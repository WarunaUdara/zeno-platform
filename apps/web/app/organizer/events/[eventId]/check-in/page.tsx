import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { connection } from "next/server";
import { OrganizerChrome } from "@/components/organizer/organizer-chrome";
import { OrganizerAuthUnavailable } from "@/components/organizer/organizer-auth-unavailable";
import { loadEventAttendance, loadOrganizerWorkspace } from "@/lib/organizer-service";
import { canRunCheckIn } from "@/lib/organizer-permissions";
import { DoorConsole } from "./door-console";
import styles from "@/components/organizer/event-editor.module.css";

export default async function CheckInPage({
  params,
  searchParams,
}: PageProps<"/organizer/events/[eventId]/check-in">) {
  await connection();
  const [{ eventId }, query] = await Promise.all([params, searchParams]);
  const requestedOrganization = typeof query.organization === "string" ? query.organization : "";
  const organizationCursor = typeof query.org_cursor === "string" ? query.org_cursor : undefined;

  const workspace = await loadOrganizerWorkspace(organizationCursor);
  if (workspace.state === "auth-not-configured") return <OrganizerAuthUnavailable />;
  if (workspace.state === "signed-out") {
    const search = new URLSearchParams();
    if (requestedOrganization) search.set("organization", requestedOrganization);
    if (organizationCursor) search.set("org_cursor", organizationCursor);
    const suffix = search.size ? `?${search.toString()}` : "";
    const returnPath = `/organizer/events/${encodeURIComponent(eventId)}/check-in${suffix}`;
    redirect(`/auth/sign-in?redirectTo=${encodeURIComponent(returnPath)}`);
  }
  if (workspace.state === "unavailable") {
    return (
      <main className={styles.editorStack}>
        <section className={styles.editorSection}>
          <p className={styles.eyebrow}>AT THE DOOR</p>
          <h1>We couldn&apos;t verify your workspace.</h1>
          <p className={styles.notice}>Try again before running the door.</p>
        </section>
      </main>
    );
  }

  const organization = workspace.organizations.find(
    (item) => item.id === requestedOrganization,
  );
  if (!organization) notFound();

  // The API enforces this too. Hiding the page keeps a volunteer from being sent
  // to a screen that refuses every scan, which at a door reads as a broken system
  // rather than a missing permission.
  if (!canRunCheckIn(organization.role)) {
    return (
      <main className={styles.editorStack}>
        <section className={styles.editorSection}>
          <p className={styles.eyebrow}>AT THE DOOR</p>
          <h1>You can&apos;t run check-in for this event.</h1>
          <p className={styles.notice}>
            Ask a workspace owner for check-in access, or ask an organizer at the door to admit
            this guest.
          </p>
        </section>
      </main>
    );
  }

  const attendanceResult = await loadEventAttendance(organization.id, eventId);
  if (attendanceResult.state === "not-found") notFound();
  if (attendanceResult.state !== "ready") {
    // A missing counter must not close the door. Staff can still scan.
    attendanceResult satisfies { state: "auth-not-configured" | "signed-out" | "unavailable" };
  }
  const attendance =
    attendanceResult.state === "ready"
      ? attendanceResult.value
      : { event_id: eventId, issued: 0, admitted: 0, awaiting: 0, voided: 0 };

  return (
    <OrganizerChrome
      email={workspace.email}
      organization={organization}
      organizationCursor={workspace.organizationCursor}
      organizationNextCursor={workspace.organizationNextCursor}
      organizations={workspace.organizations}
      title="Check-in"
    >
      <main className={styles.editorStack}>
        <DoorConsole
          eventId={eventId}
          initialAttendance={attendance}
          organizationId={organization.id}
        />
        <p className={styles.notice}>
          <Link href={`/organizer/events/${encodeURIComponent(eventId)}`}>
            Back to event setup
          </Link>
        </p>
      </main>
    </OrganizerChrome>
  );
}