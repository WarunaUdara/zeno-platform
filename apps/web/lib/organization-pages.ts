export type CursorPage<T> = { items: T[]; next_cursor: string | null };

export type OrganizationPages<T extends { id: string }> = {
  items: T[];
  cursorById: Map<string, string | null>;
  nextCursorByCursor: Map<string | null, string | null>;
};

export function createOrganizationPages<T extends { id: string }>(
  items: T[],
  cursor: string | null,
  nextCursor: string | null,
): OrganizationPages<T> {
  return addOrganizationPage(
    { items: [], cursorById: new Map(), nextCursorByCursor: new Map() },
    cursor,
    { items, next_cursor: nextCursor },
  );
}

export function addOrganizationPage<T extends { id: string }>(
  current: OrganizationPages<T>,
  cursor: string | null,
  page: CursorPage<T>,
): OrganizationPages<T> {
  const items = [...current.items];
  const itemIndex = new Map(items.map((item, index) => [item.id, index]));
  const cursorById = new Map(current.cursorById);
  for (const item of page.items) {
    const existingIndex = itemIndex.get(item.id);
    if (existingIndex === undefined) {
      itemIndex.set(item.id, items.length);
      items.push(item);
    } else {
      items[existingIndex] = item;
    }
    cursorById.set(item.id, cursor);
  }

  return {
    items,
    cursorById,
    nextCursorByCursor: new Map(current.nextCursorByCursor).set(cursor, page.next_cursor),
  };
}

export function getOrganizationCursor<T extends { id: string }>(
  pages: OrganizationPages<T>,
  organizationId: string,
  fallback: string | null,
): string | null {
  return pages.cursorById.has(organizationId)
    ? pages.cursorById.get(organizationId) ?? null
    : fallback;
}

export function getNextOrganizationCursor<T extends { id: string }>(
  pages: OrganizationPages<T>,
  cursor: string | null,
  fallback: string | null,
): string | null {
  return pages.nextCursorByCursor.has(cursor)
    ? pages.nextCursorByCursor.get(cursor) ?? null
    : fallback;
}
