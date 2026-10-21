import { redirect } from "next/navigation";
import { connection } from "next/server";
import { CreateOrganizationForm, OrganizerDashboard, OrganizerUnavailable } from "@/components/organizer/workspace";
import { OrganizerChrome } from "@/components/organizer/organizer-chrome";
import { OrganizerAuthUnavailable } from "@/components/organizer/organizer-auth-unavailable";
import { WorkspaceChooser } from "@/components/organizer/workspace-chooser";
import { getAuthenticatedOrganizerApi, loadOrganizerWorkspace, type OrganizerEventSummary } from "@/lib/organizer-service";
import { selectOrganization } from "@/lib/organization-selection";
import { canReviewEventSetup } from "@/lib/organizer-permissions";

export default async function OrganizerPage({ searchParams }: PageProps<"/organizer">) {
  await connection();
  const query = await searchParams;
  const organizationCursor = typeof query.org_cursor === "string" ? query.org_cursor : undefined;
  const workspace = await loadOrganizerWorkspace(organizationCursor);

  if (workspace.state === "auth-not-configured") return <OrganizerAuthUnavailable />;
  if (workspace.state === "signed-out") redirect("/auth/sign-in?redirectTo=%2Forganizer");
  if (workspace.state === "unavailable") {
    return <OrganizerUnavailable message="We couldn’t verify your sign-in or reach the organizer service. Your event data is safe; try again in a moment." />;
  }
  if (workspace.organizations.length === 0) return <CreateOrganizationForm name={workspace.name} />;

  const selectedId = typeof query.organization === "string" ? query.organization : "";
  const organization = selectOrganization(workspace.organizations, selectedId || undefined);
  if (query.workspace_chooser === "1" || (selectedId && !organization)) {
    return (
      <WorkspaceChooser
        cursor={workspace.organizationCursor}
        invalidSelection={Boolean(selectedId && !organization)}
        nextCursor={workspace.organizationNextCursor}
        organizations={workspace.organizations}
      />
    );
  }
  if (!organization) {
    return <OrganizerUnavailable message="No workspace is available for this page. Refresh and try again." />;
  }
  if (!canReviewEventSetup(organization.role)) {
    return <OrganizerUnavailable message="Event setup isn’t available to this workspace role yet. Ask a workspace owner for analyst access to review events or event manager access to make changes." />;
  }
  const access = await getAuthenticatedOrganizerApi();
  if (access.state !== "ready") {
    return <OrganizerUnavailable message="Your session could not be refreshed. Sign in again before changing an event." />;
  }

  let events: OrganizerEventSummary[] | undefined;
  let nextCursor: string | null = null;
  try {
    const { data, error } = await access.value.api.GET("/v1/organizations/{organization_id}/events", {
      params: { path: { organization_id: organization.id }, query: { limit: 50 } },
    });
    if (!error && data) {
      events = data.items;
      nextCursor = data.next_cursor;
    }
  } catch {
    return <OrganizerUnavailable message="We couldn’t load this workspace’s events. Check your connection and try again." />;
  }
  if (!events) return <OrganizerUnavailable message="We couldn’t load this workspace’s events. Check your connection and try again." />;

  return (
    <OrganizerChrome
      email={workspace.email}
      organization={organization}
      organizationCursor={workspace.organizationCursor}
      organizationNextCursor={workspace.organizationNextCursor}
      organizations={workspace.organizations}
      title="Events"
    >
      <OrganizerDashboard
        events={events}
        key={organization.id}
        nextCursor={nextCursor}
        organization={organization}
        organizationCursor={workspace.organizationCursor}
      />
    </OrganizerChrome>
  );
}
