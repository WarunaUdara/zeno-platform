export function suggestedWorkspaceSlug(name: string): string {
  const slug = name
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 62)
    .replace(/-$/g, "");

  return slug.length >= 3 ? slug : "workspace";
}

export function isValidWorkspaceSlug(slug: string): boolean {
  return slug.length >= 3 && slug.length <= 62 && /^[a-z0-9]([a-z0-9-]{1,60}[a-z0-9])$/.test(slug);
}
