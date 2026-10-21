import Link from "next/link";
import type { Organization } from "@/lib/organizer-service";
import { organizationChoiceHref, workspaceChooserHref } from "@/lib/workspace-chooser-links";
import styles from "./workspace.module.css";

export function WorkspaceChooser({
  organizations,
  cursor,
  nextCursor,
  invalidSelection,
}: {
  organizations: Organization[];
  cursor: string | null;
  nextCursor: string | null;
  invalidSelection: boolean;
}) {
  return (
    <main className={styles.setupMain}>
      <p className={styles.eyebrow}>WORKSPACES</p>
      <h1>Choose a workspace</h1>
      <p className={styles.intro}>
        {invalidSelection
          ? "That workspace isn’t available to your account. Choose another workspace to continue."
          : "Choose a workspace you belong to."}
      </p>
      <nav aria-label="Available workspaces">
        <ul className={styles.workspaceOptions}>
          {organizations.map((organization) => (
            <li key={organization.id}>
              <Link className={styles.workspaceOption} href={organizationChoiceHref(organization.id, cursor)}>
                <span className={styles.workspaceOptionCopy}>
                  <span className={styles.workspaceOptionName}>{organization.name}</span>
                  <span className={styles.workspaceOptionRole}>{organization.role.replaceAll("_", " ")}</span>
                </span>
                <span aria-hidden="true">→</span>
              </Link>
            </li>
          ))}
        </ul>
      </nav>
      <div className={styles.workspacePages}>
        {cursor ? <Link href={workspaceChooserHref(null)}>First workspaces</Link> : null}
        {nextCursor ? <Link href={workspaceChooserHref(nextCursor)}>More workspaces →</Link> : null}
      </div>
    </main>
  );
}
