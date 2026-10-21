export const EVENT_TIMEZONES: { value: string; label: string }[] = [
  { value: "Asia/Colombo", label: "Colombo (GMT+5:30)" },
  { value: "Asia/Dhaka", label: "Dhaka (GMT+6)" },
  { value: "Asia/Kolkata", label: "Kolkata (GMT+5:30)" },
  { value: "Asia/Karachi", label: "Karachi (GMT+5)" },
  { value: "Asia/Dubai", label: "Dubai (GMT+4)" },
  { value: "Asia/Singapore", label: "Singapore (GMT+8)" },
  { value: "Asia/Tokyo", label: "Tokyo (GMT+9)" },
  { value: "Australia/Sydney", label: "Sydney (GMT+10)" },
  { value: "Europe/London", label: "London (GMT+0)" },
  { value: "Europe/Berlin", label: "Berlin (GMT+1)" },
  { value: "America/New_York", label: "New York (GMT-5)" },
  { value: "America/Los_Angeles", label: "Los Angeles (GMT-8)" },
  { value: "America/Chicago", label: "Chicago (GMT-6)" },
  { value: "UTC", label: "UTC (GMT+0)" },
];

export const DEFAULT_EVENT_TIMEZONE = "Asia/Colombo";

export const TICKET_CURRENCIES: { value: string; label: string }[] = [
  { value: "LKR", label: "LKR — Sri Lankan rupee" },
  { value: "USD", label: "USD — US dollar" },
  { value: "EUR", label: "EUR — Euro" },
  { value: "GBP", label: "GBP — British pound" },
  { value: "SGD", label: "SGD — Singapore dollar" },
  { value: "AUD", label: "AUD — Australian dollar" },
  { value: "INR", label: "INR — Indian rupee" },
];

export const DEFAULT_TICKET_CURRENCY = "LKR";

/**
 * Match a stored IANA zone to the select, falling back to the raw value.
 *
 * Events created before this list existed, or in a zone the list does not name,
 * must keep their own zone: silently rewriting an organizer's event timezone
 * would move the start time without telling them.
 */
export function matchTimezoneOption(value: string) {
  return EVENT_TIMEZONES.find((option) => option.value === value);
}

export function matchCurrencyOption(value: string) {
  return TICKET_CURRENCIES.find((option) => option.value === value);
}