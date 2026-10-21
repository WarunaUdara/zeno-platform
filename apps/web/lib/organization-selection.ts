export function selectOrganization<T extends { id: string }>(
  organizations: T[],
  requestedId: string | undefined,
): T | undefined {
  if (!requestedId) return organizations[0];
  return organizations.find((organization) => organization.id === requestedId);
}

export function isValidOrganizationCursor(cursor: string | null | undefined): boolean {
  return cursor === undefined || cursor === null || /^[A-Za-z0-9_-]{1,512}$/.test(cursor);
}
