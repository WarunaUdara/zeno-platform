const EVENT_SETUP_READER_ROLES = new Set([
  "owner",
  "admin",
  "event_manager",
  "analyst",
]);

const EVENT_MANAGER_ROLES = new Set(["owner", "admin", "event_manager"]);

// Running a door is deliberately a narrower role than editing an event. A
// volunteer is trusted to admit guests and nothing else, which keeps the blast
// radius of a borrowed phone at the entrance small.
const CHECK_IN_ROLES = new Set([
  "owner",
  "admin",
  "event_manager",
  "check_in_manager",
]);

export function canReviewEventSetup(role: string): boolean {
  return EVENT_SETUP_READER_ROLES.has(role);
}

export function canManageEvents(role: string): boolean {
  return EVENT_MANAGER_ROLES.has(role);
}

export function canRunCheckIn(role: string): boolean {
  return CHECK_IN_ROLES.has(role);
}
