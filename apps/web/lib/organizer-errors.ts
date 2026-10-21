function commonErrorForStatus(status: number, fallback: string) {
  if (status === 401) return "Your session ended. Sign in again to continue.";
  if (status === 404) return "This workspace or event is no longer available.";
  if (status === 409) return "This change conflicts with the current event. Refresh the page and review your details.";
  return fallback;
}

export function organizerFormError(status: number, fallback: string) {
  if (status === 422) return "Some details weren’t accepted. Review your entries and try again.";
  return commonErrorForStatus(status, fallback);
}

export function organizerListError(status: number, fallback: string) {
  if (status === 422) return "This list changed while you were viewing it. Refresh the page to load the latest results.";
  return commonErrorForStatus(status, fallback);
}
