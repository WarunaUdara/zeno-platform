"use client";

import Link from "next/link";
import { NeonAuthUIProvider, AuthView } from "@neondatabase/auth-ui";
import type { AuthViewPath } from "@neondatabase/auth-ui/server";
import { authClient } from "@/lib/auth/client";
import styles from "./auth-portal.module.css";

export function AuthPortal({ path, redirectTo }: { path: AuthViewPath; redirectTo: string }) {
  return (
    <NeonAuthUIProvider authClient={authClient} defaultTheme="dark" redirectTo={redirectTo}>
      <main className={styles.page}>
        <Link className={styles.brand} href="/">
          <span aria-hidden="true" className={styles.mark}>z</span>
          <span>Zeno<span className={styles.period}>.</span></span>
        </Link>
        <div className={styles.content}>
          <AuthView view={path} />
          <p className={styles.footnote}>One account for the event workspaces you belong to.</p>
        </div>
      </main>
    </NeonAuthUIProvider>
  );
}
