import { notFound, redirect } from "next/navigation";
import { connection } from "next/server";
import { OrganizerEventEditor } from "@/components/organizer/event-editor";
import { OrganizerChrome } from "@/components/organizer/organizer-chrome";
import { OrganizerAuthUnavailable } from "@/components/organizer/organizer-auth-unavailable";
import { OrganizerUnavailable } from "@/components/organizer/workspace";
import { loadOrganizerEvent, loadOrganizerWorkspace } from "@/lib/organizer-service";
import { canManageEvents, canReviewEventSetup } from "@/lib/organizer-permissions";
import styles from "@/components/organizer/event-editor.module.css";

export default async function OrganizerEventPage({
  params,
  searchParams,
}: PageProps<"/organizer/events/[eventId]">) {
  await connection();
  const [{ eventId }, query] = await Promise.all([params, searchParams]);
  const organizationCursor = typeof query.org_cursor === "string" ? query.org_cursor : undefined;
  const workspace = await loadOrganizerWorkspace(organizationCursor);
  if (workspace.state === "auth-not-configured") return <OrganizerAuthUnavailable />;
  if (workspace.state === "signed-out") {
    const search = new URLSearchParams();
    if (typeof query.organization === "string") search.set("organization", query.organization);
    if (organizationCursor) search.set("org_cursor", organizationCursor);
    const organizationQuery = search.size ? `?${search.toString()}` : "";
    const returnPath = `/organizer/events/${encodeURIComponent(eventId)}${organizationQuery}`;
    redirect(`/auth/sign-in?redirectTo=${encodeURIComponent(returnPath)}`);
  }
  if (workspace.state === "unavailable") {
    return <OrganizerUnavailable message="We couldn’t verify your workspace. Try again before changing an event." />;
  }

  const requestedOrganization = typeof query.organization === "string" ? query.organization : "";
  const organization = workspace.organizations.find((item) => item.id === requestedOrganization);
  if (!organization) notFound();
  if (!canReviewEventSetup(organization.role)) {
    return <OrganizerUnavailable message="Event setup isn’t available to this workspace role yet. Ask a workspace owner for analyst access to review events or event manager access to make changes." />;
  }

  const result = await loadOrganizerEvent(organization.id, eventId);
  if (result.state === "not-found") notFound();
  if (result.state !== "ready") {
    return <OrganizerUnavailable message="We couldn’t load this event. Check your connection and try again." />;
  }

  return (
    <OrganizerChrome
      email={workspace.email}
      organization={organization}
      organizationCursor={workspace.organizationCursor}
      organizationNextCursor={workspace.organizationNextCursor}
      organizations={workspace.organizations}
      title={result.event.title}
    >
      <div className={styles.editorContent}>
        <p className={styles.eyebrow}>{result.event.visibility === "public" ? "EVENT PAGE" : "STEP 2 OF 2"}</p>
        <h1>{result.event.visibility === "public" ? result.event.title : canManageEvents(organization.role) ? "Finish setting up your event" : "Review event draft"}</h1>
        <p className={styles.lede}>{canManageEvents(organization.role)
          ? result.event.visibility === "public" ? "Review the event information guests can see." : "Add at least one ticket option, then publish when everything looks right."
          : "You have read-only access. Ask a workspace owner for event manager access to make changes."}</p>
        <OrganizerEventEditor
          canEdit={canManageEvents(organization.role)}
          event={result.event}
          now={new Date().toISOString()}
          organizationId={organization.id}
          tickets={result.tickets}
        />
      </div>
    </OrganizerChrome>
  );
}
