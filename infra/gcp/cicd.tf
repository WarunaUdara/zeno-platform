# Deployment identity for GitHub Actions, using Workload Identity Federation.
#
# There is deliberately no service-account JSON key anywhere in this project.
# A key is a bearer credential: it does not expire, it survives in a workflow
# log or a laptop backup, and rotating it is a manual ceremony that people
# forget. Instead GitHub mints a short-lived OIDC token for the workflow run and
# Google exchanges it for credentials that die with the job, so a leaked log
# yields nothing reusable.
#
# The exchange is only allowed for one repository. A fork, or anyone who copies
# the workflow file, gets a token whose `repository` claim does not match the
# attribute condition below, and the impersonation is refused.

locals {
  github_deploy_enabled = trimspace(var.github_repository) != ""
}

variable "github_repository" {
  description = "GitHub repository allowed to deploy, as owner/name. Empty disables the CI/CD identity entirely, so a clone that does not set it never creates a trust relationship."
  type        = string
  default     = ""

  validation {
    condition = var.github_repository == "" || can(regex(
      "^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$",
      var.github_repository,
    ))
    error_message = "github_repository must be empty or owner/name using only letters, digits, dots, underscores, and hyphens."
  }
}

resource "google_project_service" "workload_identity" {
  for_each = local.github_deploy_enabled ? toset([
    "iamcredentials.googleapis.com",
    "sts.googleapis.com",
  ]) : toset([])

  project            = var.project_id
  service            = each.value
  disable_on_destroy = false
}

resource "google_iam_workload_identity_pool" "github" {
  count = local.github_deploy_enabled ? 1 : 0

  project                   = var.project_id
  workload_identity_pool_id = "${local.prefix}-github"
  display_name              = "GitHub Actions (${var.environment})"
  description               = "Federated identity for ${var.github_repository} workflow runs. No keys are issued."

  depends_on = [google_project_service.required, google_project_service.workload_identity]
}

resource "google_iam_workload_identity_pool_provider" "github" {
  count = local.github_deploy_enabled ? 1 : 0

  project                            = var.project_id
  workload_identity_pool_id          = google_iam_workload_identity_pool.github[0].workload_identity_pool_id
  workload_identity_pool_provider_id = "github"
  display_name                       = "GitHub OIDC"

  oidc {
    issuer_uri = "https://token.actions.githubusercontent.com"
  }

  attribute_mapping = {
    "google.subject"        = "assertion.sub"
    "attribute.repository"  = "assertion.repository"
    "attribute.ref"         = "assertion.ref"
    "attribute.environment" = "assertion.environment"
    "attribute.event_name"  = "assertion.event_name"
    "attribute.actor"       = "assertion.actor"
  }

  # Pin both the source control identity and the delivery context. Repository
  # alone is not enough: a pull_request run from a fork carries the base
  # repository claim, and a workflow_dispatch run can target any branch. The
  # deploy workflow uses the `production` environment on a push to main, so
  # those are the only claims that should be able to mint deploy credentials.
  attribute_condition = join(" && ", [
    "assertion.repository == '${var.github_repository}'",
    "assertion.ref == 'refs/heads/main'",
    "assertion.environment == 'production'",
  ])
}

resource "google_service_account" "github_deploy" {
  count = local.github_deploy_enabled ? 1 : 0

  project      = var.project_id
  account_id   = "${local.prefix}-github-deploy"
  display_name = "Zeno GitHub Actions deploy (${var.environment})"
  description  = "Builds and rolls out images for ${var.github_repository}. Bound to a push to main in the production environment."
}

# The pool may impersonate exactly this one service account, and only for
# principals arriving through the repository-scoped attribute above.
resource "google_service_account_iam_member" "github_deploy_federation" {
  count = local.github_deploy_enabled ? 1 : 0

  service_account_id = google_service_account.github_deploy[0].name
  role               = "roles/iam.workloadIdentityUser"
  member             = "principalSet://iam.googleapis.com/${google_iam_workload_identity_pool.github[0].name}/attribute.repository/${var.github_repository}"
}

# Least privilege for a deploy, and deliberately not roles/editor or owner.
#   run.developer      - create and update services and jobs. Note this is not
#                        run.admin: the deploy identity cannot change a service's
#                        IAM policy, so it cannot grant itself or anyone else
#                        access to a running service.
#   artifactregistry.writer - push image tags.
#   serviceusage.services.use - required for any API call against the project.
resource "google_project_iam_member" "github_deploy_roles" {
  for_each = local.github_deploy_enabled ? toset([
    "roles/run.developer",
    "roles/artifactregistry.writer",
    "roles/serviceusage.serviceUsageConsumer",
  ]) : toset([])

  project = var.project_id
  role    = each.value
  member  = "serviceAccount:${google_service_account.github_deploy[0].email}"
}

# Deploying a Cloud Run resource that runs as a service account requires the
# deployer to be able to act as that account. Scoped per-account rather than
# project-wide, so the deploy identity cannot act as the migration account
# outside the job it is meant to update.
resource "google_service_account_iam_member" "github_deploy_acts_as" {
  for_each = local.github_deploy_enabled ? {
    api    = google_service_account.api.email
    web    = google_service_account.web.email
    worker = google_service_account.worker.email
  } : {}

  service_account_id = "projects/${var.project_id}/serviceAccounts/${each.value}"
  role               = "roles/iam.serviceAccountUser"
  member             = "serviceAccount:${google_service_account.github_deploy[0].email}"
}

# The migration job runs as its own identity, and the deploy rolls that job's
# image forward with each release, so it needs the same permission.
resource "google_service_account_iam_member" "github_deploy_acts_as_migration" {
  count = local.github_deploy_enabled ? 1 : 0

  service_account_id = "projects/${var.project_id}/serviceAccounts/${google_service_account.migration.email}"
  role               = "roles/iam.serviceAccountUser"
  member             = "serviceAccount:${google_service_account.github_deploy[0].email}"
}
