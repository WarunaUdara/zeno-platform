//! Validation of registration form definitions and the answers submitted against
//! them.
//!
//! Forms and answers are validated here rather than in the HTTP layer because the
//! same rules must hold whether an answer arrives from the public API, from a
//! server action, or from a background job. Validation is a pure function over a
//! loaded definition so it can be exhaustively unit tested without a database.

use serde::{Deserialize, Serialize};

pub const MAX_FIELDS_PER_FORM: usize = 50;
pub const MAX_OPTIONS_PER_CHOICE: usize = 50;
pub const MAX_ANSWER_LENGTH: usize = 2000;
const MAX_MULTI_CHOICE_ANSWERS: usize = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    ShortText,
    LongText,
    Email,
    Phone,
    Number,
    SingleChoice,
    MultiChoice,
    Checkbox,
}

impl FieldType {
    pub fn expects_options(self) -> bool {
        matches!(self, Self::SingleChoice | Self::MultiChoice)
    }

    fn is_multiline(self) -> bool {
        matches!(self, Self::LongText)
    }
}

impl FieldType {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "short_text" => Some(Self::ShortText),
            "long_text" => Some(Self::LongText),
            "email" => Some(Self::Email),
            "phone" => Some(Self::Phone),
            "number" => Some(Self::Number),
            "single_choice" => Some(Self::SingleChoice),
            "multi_choice" => Some(Self::MultiChoice),
            "checkbox" => Some(Self::Checkbox),
            _ => None,
        }
    }
}

/// A field as stored and as submitted.
#[derive(Clone, Debug)]
pub struct FieldDefinition {
    pub id: String,
    pub field_key: String,
    pub label: String,
    pub field_type: FieldType,
    pub required: bool,
    pub options: Vec<String>,
}

/// One submitted answer, matched to its field by `field_key`.
#[derive(Clone, Debug)]
pub struct SubmittedAnswer {
    pub field_key: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerRejection {
    UnknownField,
    DuplicateAnswer,
    MissingRequired,
    TooLong,
    EmptyValue,
    NotAnEmail,
    NotANumber,
    NotAChoice,
    TooManyChoices,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnswerProblem {
    pub field_key: String,
    pub rejection: AnswerRejection,
}

/// Validate a set of submitted answers against a form definition.
///
/// Returns every problem rather than the first, so the attendee sees all their
/// mistakes at once instead of discovering them one resubmission at a time.
pub fn validate_answers(
    fields: &[FieldDefinition],
    submitted: &[SubmittedAnswer],
) -> Vec<AnswerProblem> {
    let mut problems = Vec::new();

    for answer in submitted {
        if !fields
            .iter()
            .any(|field| field.field_key == answer.field_key)
        {
            // An answer to a question that is not on this form is a contract
            // violation, not a validation nit: it would be stored unreadable.
            problems.push(AnswerProblem {
                field_key: answer.field_key.clone(),
                rejection: AnswerRejection::UnknownField,
            });
        }
    }

    // A repeated question key means the client sent the same question twice. The
    // unique index would reject it at insert time, so reject it here with a clear
    // reason instead of letting it surface as a constraint violation. Multi-choice
    // is the exception: it is submitted as several rows under one key by design.
    let mut seen = std::collections::HashSet::new();
    for answer in submitted {
        let is_multi_choice = fields.iter().any(|field| {
            field.field_key == answer.field_key && field.field_type == FieldType::MultiChoice
        });
        if !is_multi_choice && !seen.insert(answer.field_key.as_str()) {
            problems.push(AnswerProblem {
                field_key: answer.field_key.clone(),
                rejection: AnswerRejection::DuplicateAnswer,
            });
        }
    }

    for field in fields {
        let values = submitted
            .iter()
            .filter(|answer| answer.field_key == field.field_key)
            .map(|answer| answer.value.as_str())
            .collect::<Vec<_>>();

        match field.field_type {
            FieldType::MultiChoice => {
                if values.is_empty() {
                    if field.required {
                        problems.push(problem(field, AnswerRejection::MissingRequired));
                    }
                    continue;
                }
                if values.len() > MAX_MULTI_CHOICE_ANSWERS {
                    problems.push(problem(field, AnswerRejection::TooManyChoices));
                    continue;
                }
                for value in values {
                    if !field.options.iter().any(|option| option == value) {
                        problems.push(problem(field, AnswerRejection::NotAChoice));
                        break;
                    }
                }
            }
            _ => {
                // Everything except multi-choice collapses to a single value, so a
                // duplicate is reported rather than silently taking the first.
                if let Some(value) = values.first() {
                    if values.len() > 1 {
                        problems.push(problem(field, AnswerRejection::DuplicateAnswer));
                    }
                    validate_single_value(field, value, &mut problems);
                } else if field.required {
                    problems.push(problem(field, AnswerRejection::MissingRequired));
                }
            }
        }
    }

    problems
}

fn validate_single_value(field: &FieldDefinition, value: &str, problems: &mut Vec<AnswerProblem>) {
    let value = value.trim();
    if value.is_empty() {
        // An optional question answered with whitespace is treated as unanswered,
        // which is what a person typing a space meant.
        if field.required {
            problems.push(problem(field, AnswerRejection::EmptyValue));
        }
        return;
    }
    if value.chars().count() > MAX_ANSWER_LENGTH {
        problems.push(problem(field, AnswerRejection::TooLong));
        return;
    }
    match field.field_type {
        FieldType::Checkbox => {
            if !matches!(value, "true" | "false") {
                problems.push(problem(field, AnswerRejection::NotAChoice));
            }
        }
        FieldType::Email => {
            if !is_plausible_email(value) {
                problems.push(problem(field, AnswerRejection::NotAnEmail));
            }
        }
        FieldType::Number => {
            if value.parse::<f64>().is_err() {
                problems.push(problem(field, AnswerRejection::NotANumber));
            }
        }
        FieldType::Phone => {
            if !value
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | ' ' | '(' | ')'))
                || value.chars().filter(|c| c.is_ascii_digit()).count() < 5
            {
                problems.push(problem(field, AnswerRejection::NotANumber));
            }
        }
        FieldType::SingleChoice => {
            if !field.options.iter().any(|option| option == value) {
                problems.push(problem(field, AnswerRejection::NotAChoice));
            }
        }
        FieldType::LongText => {}
        FieldType::ShortText | FieldType::MultiChoice => {}
    }
}

fn problem(field: &FieldDefinition, rejection: AnswerRejection) -> AnswerProblem {
    AnswerProblem {
        field_key: field.field_key.clone(),
        rejection,
    }
}

/// Structural validation of an authored field, applied before it is stored.
pub fn validate_field(
    field_key: &str,
    label: &str,
    field_type: FieldType,
    required: bool,
    options: &[String],
) -> bool {
    let key_ok = (1..=40).contains(&field_key.len())
        && field_key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && !field_key.starts_with('_')
        && !field_key.ends_with('_');
    let label_ok =
        (1..=160).contains(&label.trim().chars().count()) && !label.chars().any(char::is_control);
    let options_ok = if field_type.expects_options() {
        !options.is_empty()
            && options.len() <= MAX_OPTIONS_PER_CHOICE
            && options
                .iter()
                .all(|option| (1..=120).contains(&option.trim().chars().count()))
            && options
                .iter()
                .map(|option| option.trim())
                .collect::<std::collections::HashSet<_>>()
                .len()
                == options.len()
    } else {
        options.is_empty()
    };
    // A required checkbox nobody can answer is a trap: the attendee cannot clear it.
    let required_ok = !(required && field_type == FieldType::Checkbox);
    key_ok && label_ok && options_ok && required_ok
}

/// Deliberately permissive. The goal is to reject an obvious typo, not to enforce
/// RFC 5322; rejecting a real address is far more damaging than accepting a
/// malformed one, and the value is never used to send anything yet.
fn is_plausible_email(value: &str) -> bool {
    let (local, domain) = match value.split_once('@') {
        Some(parts) => parts,
        None => return false,
    };
    !local.is_empty()
        && local.len() <= 64
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !value.contains(char::is_whitespace)
        && value.len() <= 254
}

/// Does this field type keep a multi-line answer readable, or must newlines be
/// stripped so the value stays one line everywhere it is later displayed?
pub fn allows_multiline(field_type: FieldType) -> bool {
    field_type.is_multiline()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(
        key: &str,
        field_type: FieldType,
        required: bool,
        options: &[&str],
    ) -> FieldDefinition {
        FieldDefinition {
            id: format!("zeno_{key}00000000000000000000000000"),
            field_key: key.to_owned(),
            label: key.to_owned(),
            field_type,
            required,
            options: options.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    fn answers(pairs: &[(&str, &str)]) -> Vec<SubmittedAnswer> {
        pairs
            .iter()
            .map(|(key, value)| SubmittedAnswer {
                field_key: (*key).to_owned(),
                value: (*value).to_owned(),
            })
            .collect()
    }

    fn opts(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn a_complete_valid_submission_passes() {
        let fields = vec![
            field("team", FieldType::SingleChoice, true, &["Eng", "Design"]),
            field("email", FieldType::Email, true, &[]),
            field("diet", FieldType::MultiChoice, false, &["Vegan", "None"]),
            field("note", FieldType::LongText, false, &[]),
        ];
        let submitted = answers(&[
            ("team", "Eng"),
            ("email", "guest@example.com"),
            ("diet", "Vegan"),
            ("note", "Wheelchair access needed"),
        ]);
        assert!(validate_answers(&fields, &submitted).is_empty());
    }

    #[test]
    fn every_problem_is_reported_not_just_the_first() {
        // An attendee fixing one field at a time would otherwise need three
        // resubmissions to fix three mistakes.
        let fields = vec![
            field("email", FieldType::Email, true, &[]),
            field("team", FieldType::SingleChoice, true, &["Eng"]),
            field("age", FieldType::Number, true, &[]),
        ];
        let submitted = answers(&[("email", "nope"), ("team", "Legal"), ("age", "old")]);
        let problems = validate_answers(&fields, &submitted);
        assert_eq!(problems.len(), 3, "expected all three problems at once");
    }

    #[test]
    fn a_missing_required_answer_is_rejected_but_an_optional_blank_is_not() {
        let fields = vec![
            field("team", FieldType::SingleChoice, true, &["Eng"]),
            field("note", FieldType::ShortText, false, &[]),
        ];
        let problems = validate_answers(&fields, &[]);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].field_key, "team");
        assert_eq!(problems[0].rejection, AnswerRejection::MissingRequired);

        // Whitespace in an optional answer means "no answer", not "a bad answer".
        assert!(
            validate_answers(&fields, &answers(&[("note", "   ")]))
                .iter()
                .all(|problem| problem.rejection != AnswerRejection::EmptyValue)
        );
    }

    #[test]
    fn an_answer_to_a_question_that_is_not_on_the_form_is_rejected() {
        // Storing it would produce a row no reader can display, so this is a hard
        // rejection rather than a warning.
        let fields = vec![field("team", FieldType::ShortText, false, &[])];
        let problems = validate_answers(&fields, &answers(&[("smuggled", "value")]));
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].rejection, AnswerRejection::UnknownField);
    }

    #[test]
    fn a_choice_outside_the_offered_options_is_rejected() {
        let fields = vec![field(
            "team",
            FieldType::SingleChoice,
            true,
            &["Eng", "Design"],
        )];
        let problems = validate_answers(&fields, &answers(&[("team", "Legal")]));
        assert_eq!(problems[0].rejection, AnswerRejection::NotAChoice);
    }

    #[test]
    fn multi_choice_accepts_several_options_and_rejects_nonexistent_ones() {
        let fields = vec![field(
            "diet",
            FieldType::MultiChoice,
            false,
            &["Vegan", "GF", "None"],
        )];
        assert!(
            validate_answers(&fields, &answers(&[("diet", "Vegan"), ("diet", "GF")])).is_empty()
        );
        assert_eq!(
            validate_answers(&fields, &answers(&[("diet", "Vegan"), ("diet", "Kosher")]))[0]
                .rejection,
            AnswerRejection::NotAChoice
        );
    }

    #[test]
    fn a_repeated_question_is_rejected_rather_than_silently_truncated() {
        // The unique index would otherwise reject it at insert time and the caller
        // would see an opaque constraint violation.
        let fields = vec![field("note", FieldType::ShortText, false, &[])];
        let problems = validate_answers(&fields, &answers(&[("note", "a"), ("note", "b")]));
        assert_eq!(problems[0].rejection, AnswerRejection::DuplicateAnswer);
    }

    #[test]
    fn answer_length_is_capped() {
        let fields = vec![field("note", FieldType::LongText, false, &[])];
        let long = "x".repeat(MAX_ANSWER_LENGTH + 1);
        assert_eq!(
            validate_answers(&fields, &answers(&[("note", &long)]))[0].rejection,
            AnswerRejection::TooLong
        );
    }

    #[test]
    fn phone_numbers_accept_common_shapes_and_reject_free_text() {
        let fields = vec![field("phone", FieldType::Phone, false, &[])];
        for good in ["+94 77 123 4567", "(011) 234-5678", "0771234567"] {
            assert!(
                validate_answers(&fields, &answers(&[("phone", good)])).is_empty(),
                "expected {good} to be accepted"
            );
        }
        for bad in ["call me", "123", "077-CALL-NOW"] {
            assert!(
                !validate_answers(&fields, &answers(&[("phone", bad)])).is_empty(),
                "expected {bad} to be rejected"
            );
        }
    }

    #[test]
    fn a_required_checkbox_question_is_not_allowed() {
        // Nobody can clear a required checkbox, so it is always unsatisfiable.
        assert!(!validate_field(
            "agree",
            "I agree",
            FieldType::Checkbox,
            true,
            &[]
        ));
        assert!(validate_field(
            "agree",
            "I agree",
            FieldType::Checkbox,
            false,
            &[]
        ));
    }

    #[test]
    fn choice_questions_need_distinct_bounded_options() {
        assert!(validate_field(
            "team",
            "Team",
            FieldType::SingleChoice,
            false,
            &opts(&["Eng", "Design"])
        ));
        assert!(!validate_field(
            "team",
            "Team",
            FieldType::SingleChoice,
            false,
            &[]
        ));
        assert!(!validate_field(
            "team",
            "Team",
            FieldType::SingleChoice,
            false,
            &opts(&["Eng", "Eng"])
        ));
        // A choice list on a free-text question is a modelling error the database
        // constraint also rejects.
        assert!(!validate_field(
            "team",
            "Team",
            FieldType::ShortText,
            false,
            &opts(&["Eng"])
        ));
    }

    #[test]
    fn field_keys_must_be_stable_identifiers() {
        for bad in ["", "Team", "my team", "-team", "team-", "_team", "team_"] {
            assert!(
                !validate_field(bad, "Team", FieldType::ShortText, false, &[]),
                "expected {bad:?} to be rejected"
            );
        }
        assert!(validate_field(
            "team_name",
            "Team",
            FieldType::ShortText,
            false,
            &[]
        ));
    }

    #[test]
    fn email_validation_rejects_obvious_typos_without_being_presumptuous() {
        let fields = vec![field("email", FieldType::Email, false, &[])];
        for bad in ["nope", "a@b", "a b@example.com", "@example.com", "a@.com"] {
            assert!(
                !validate_answers(&fields, &answers(&[("email", bad)])).is_empty(),
                "expected {bad:?} to be rejected"
            );
        }
        for good in ["a@example.com", "first.last+tag@sub.example.co.uk"] {
            assert!(
                validate_answers(&fields, &answers(&[("email", good)])).is_empty(),
                "expected {good:?} to be accepted"
            );
        }
    }
}
