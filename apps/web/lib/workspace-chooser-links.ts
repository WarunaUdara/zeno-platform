export function workspaceChooserHref(cursor: string | null) {
  const query = new URLSearchParams({ workspace_chooser: "1" });
  if (cursor) query.set("org_cursor", cursor);
  return `/organizer?${query.toString()}`;
}

export function organizationChoiceHref(organizationId: string, cursor: string | null) {
  const query = new URLSearchParams({ organization: organizationId });
  if (cursor) query.set("org_cursor", cursor);
  return `/organizer?${query.toString()}`;
}
