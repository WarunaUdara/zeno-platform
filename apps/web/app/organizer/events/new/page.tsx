import { redirect } from "next/navigation";
import { connection } from "next/server";
import { EventDraftForm } from "@/components/organizer/event-draft-form";
import { OrganizerChrome } from "@/components/organizer/organizer-chrome";
import { OrganizerAuthUnavailable } from "@/components/organizer/organizer-auth-unavailable";
import { OrganizerUnavailable } from "@/components/organizer/workspace";
import { loadOrganizerWorkspace } from "@/lib/organizer-service";
import { selectOrganization } from "@/lib/organization-selection";
import { canManageEvents } from "@/lib/organizer-permissions";
import styles from "@/components/organizer/event-editor.module.css";

export default async function NewOrganizerEventPage({ searchParams }: PageProps<"/organizer/events/new">) {
  await connection();
  const query = await searchParams;
  const organizationCursor = typeof query.org_cursor === "string" ? query.org_cursor : undefined;
  const workspace = await loadOrganizerWorkspace(organizationCursor);
  if (workspace.state === "auth-not-configured") return <OrganizerAuthUnavailable />;
  if (workspace.state === "signed-out") {
    const search = new URLSearchParams();
    if (typeof query.organization === "string") search.set("organization", query.organization);
    if (organizationCursor) search.set("org_cursor", organizationCursor);
    const organizationQuery = search.size ? `?${search.toString()}` : "";
    const returnPath = `/organizer/events/new${organizationQuery}`;
    redirect(`/auth/sign-in?redirectTo=${encodeURIComponent(returnPath)}`);
  }
  if (workspace.state === "unavailable") {
    return <OrganizerUnavailable message="We couldn’t verify your workspace. Try again before creating an event." />;
  }

  const requested = typeof query.organization === "string" ? query.organization : "";
  const organization = selectOrganization(workspace.organizations, requested || undefined);
  if (requested && !organization) {
    return <OrganizerUnavailable message="That workspace isn’t available to your account. Choose a workspace you belong to and try again." />;
  }
  if (!organization) redirect("/organizer");
  if (!canManageEvents(organization.role)) {
    return <OrganizerUnavailable message="Your workspace role can’t create events. Ask a workspace owner for event manager access." />;
  }

  return (
    <OrganizerChrome
      email={workspace.email}
      organization={organization}
      organizationCursor={workspace.organizationCursor}
      organizationNextCursor={workspace.organizationNextCursor}
      organizations={workspace.organizations}
      title="New event"
    >
      <div className={styles.editorContent}>
        <p className={styles.eyebrow}>STEP 1 OF 2</p>
        <h1>Create an event</h1>
        <p className={styles.lede}>Add the details guests need to decide if this event is for them.</p>
        <EventDraftForm organizationCursor={workspace.organizationCursor} organizationId={organization.id} />
      </div>
    </OrganizerChrome>
  );
}
