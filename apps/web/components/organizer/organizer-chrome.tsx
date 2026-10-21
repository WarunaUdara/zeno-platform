"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useMemo, useState, useTransition, type ReactNode } from "react";
import { loadOrganizerOrganizationPage } from "@/app/organizer/actions";
import type { Organization } from "@/lib/organizer-service";
import {
  addOrganizationPage,
  createOrganizationPages,
  getNextOrganizationCursor,
  getOrganizationCursor,
} from "@/lib/organization-pages";
import { Icon } from "@/components/overview/icons";
import { SignOutButton } from "@/components/organizer/workspace";
import styles from "./organizer-chrome.module.css";

export function OrganizerChrome({
  organizations,
  organization,
  organizationCursor,
  organizationNextCursor,
  email,
  title,
  children,
}: {
  organizations: Organization[];
  organization: Organization;
  organizationCursor: string | null;
  organizationNextCursor: string | null;
  email: string;
  title: string;
  children: ReactNode;
}) {
  const pathname = usePathname();
  const eventsActive = pathname === "/organizer" || pathname.startsWith("/organizer/events");
  const [loadedOrganizationPages, setLoadedOrganizationPages] = useState(
    () => createOrganizationPages(organizations, organizationCursor, organizationNextCursor),
  );
  const [selectionState, setSelectionState] = useState({
    organizationId: organization.id,
    selectedId: organization.id,
  });
  const [loadingOrganizations, startLoadingOrganizations] = useTransition();
  const [organizationLoadMessage, setOrganizationLoadMessage] = useState("");

  const organizationPages = useMemo(
    () => addOrganizationPage(loadedOrganizationPages, organizationCursor, {
      items: organizations,
      next_cursor: organizationNextCursor,
    }),
    [loadedOrganizationPages, organizationCursor, organizationNextCursor, organizations],
  );
  const organizationOptions = organizationPages.items;
  const selectedOrganizationId = selectionState.organizationId === organization.id
    ? selectionState.selectedId
    : organization.id;
  const selectedOrganizationCursor = getOrganizationCursor(organizationPages, selectedOrganizationId, organizationCursor);
  const nextOrganizationCursor = getNextOrganizationCursor(organizationPages, organizationCursor, organizationNextCursor);
  const hasFirstOrganizationPage = Array.from(organizationPages.cursorById.values()).some((cursor) => cursor === null);
  const canLoadFirstOrganizationPage = organizationCursor !== null && !hasFirstOrganizationPage;

  function loadOrganizationPage(cursor: string | null) {
    setOrganizationLoadMessage("");
    startLoadingOrganizations(async () => {
      const result = await loadOrganizerOrganizationPage(cursor);
      if (!result.ok) {
        setOrganizationLoadMessage(result.message);
        return;
      }
      setLoadedOrganizationPages((current) => addOrganizationPage(current, result.cursor, result.page));
      setOrganizationLoadMessage(`${result.page.items.length} more workspaces loaded.`);
    });
  }

  const eventsHref = `/organizer?organization=${encodeURIComponent(organization.id)}${organizationCursor ? `&org_cursor=${encodeURIComponent(organizationCursor)}` : ""}`;

  return (
    <div className={styles.app}>
      <a className={styles.skipLink} href="#organizer-main">Skip to workspace</a>
      <aside aria-label="Organizer navigation" className={styles.sidebar}>
        <Link aria-label="Zeno home" className={styles.brand} href="/">
          <span aria-hidden="true" className={styles.brandMark}>z</span>
          <span>Zeno<span className={styles.brandPeriod}>.</span></span>
        </Link>

        <div className={styles.workspacePicker}>
          <label className={styles.sectionLabel} htmlFor="organizer-workspace">WORKSPACE</label>
          {organizationOptions.length > 1 || organizationCursor !== null || nextOrganizationCursor !== null ? (
            <form action="/organizer" className={styles.workspaceForm} method="get">
              <select aria-label="Choose workspace" id="organizer-workspace" name="organization" onChange={(event) => setSelectionState({ organizationId: organization.id, selectedId: event.target.value })} value={selectedOrganizationId}>
                {organizationOptions.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
              </select>
              <input name={selectedOrganizationCursor ? "org_cursor" : undefined} type="hidden" value={selectedOrganizationCursor ?? ""} />
              <button type="submit">Switch</button>
            </form>
          ) : (
            <p className={styles.workspaceName} title={organization.name}>{organization.name}</p>
          )}
          {nextOrganizationCursor || canLoadFirstOrganizationPage ? (
            <button
              className={styles.workspaceLoadMore}
              disabled={loadingOrganizations}
              onClick={() => loadOrganizationPage(nextOrganizationCursor ?? null)}
              type="button"
            >
              {loadingOrganizations ? "Loading…" : nextOrganizationCursor ? "Load more workspaces" : "Load first 50 workspaces"}
            </button>
          ) : null}
          {organizationLoadMessage ? <span aria-live="polite" className={styles.workspaceLoadMessage} role="status">{organizationLoadMessage}</span> : null}
          <span className={styles.role}>{organization.role.replaceAll("_", " ")}</span>
        </div>

        <nav aria-label="Workspace" className={styles.navigation}>
          <p className={styles.sectionLabel}>MANAGE</p>
          <Link aria-current={eventsActive ? "page" : undefined} className={`${styles.navLink} ${eventsActive ? styles.navLinkActive : ""}`} href={eventsHref}>
            <Icon name="calendar" size={17} /><span>Events</span>
          </Link>
          <p className={styles.sectionLabel}>PREVIEW</p>
          <Link className={styles.navLink} href="/events">
            <Icon name="ticket" size={17} /><span>Attendee experience</span>
          </Link>
        </nav>

        <div className={styles.sidebarFooter}>
          <span className={styles.environment}><span aria-hidden="true" />Organizer workspace</span>
          <div className={styles.profile}>
            <span aria-hidden="true" className={styles.avatar}>{email.trim().charAt(0).toUpperCase() || "U"}</span>
            <span className={styles.email} title={email}>{email}</span>
            <SignOutButton />
          </div>
        </div>
      </aside>

      <div className={styles.mainColumn}>
        <header className={styles.topbar}>
          <div aria-label="Breadcrumb" className={styles.breadcrumb}>
            <span>{organization.name}</span><span aria-hidden="true">/</span><strong>{title}</strong>
          </div>
          <Link className={styles.topbarPreview} href="/events">View public events <Icon name="arrow-up-right" size={15} /></Link>
        </header>
        <main className={styles.content} id="organizer-main">{children}</main>
      </div>
    </div>
  );
}
