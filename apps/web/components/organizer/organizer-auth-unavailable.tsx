import { Button } from "@/components/ui/button";
import styles from "./workspace.module.css";

export function OrganizerAuthUnavailable() {
  return (
    <main className={styles.setupMain}>
      <p className={styles.eyebrow}>ORGANIZER ACCESS</p>
      <h1>Organizer sign-in isn’t available here yet.</h1>
      <p className={styles.intro}>You can still browse public events and reserve a temporary hold.</p>
      <Button href="/events" size="lg">Browse public events</Button>
    </main>
  );
}
