import { connection } from "next/server";
import type { AuthViewPath } from "@neondatabase/auth-ui/server";
import { AuthPortal } from "@/components/auth/auth-portal";
import { OrganizerAuthUnavailable } from "@/components/organizer/organizer-auth-unavailable";
import { isNeonAuthConfigured } from "@/lib/auth/server";

const allowedViews: Record<string, AuthViewPath> = {
  "sign-in": "SIGN_IN",
  "sign-up": "SIGN_UP",
  "forgot-password": "FORGOT_PASSWORD",
  "reset-password": "RESET_PASSWORD",
  "email-verification": "EMAIL_VERIFICATION",
};

function safeRedirectTo(value: string | string[] | undefined) {
  if (typeof value !== "string" || !value.startsWith("/") || value.startsWith("//") || value.includes("\\")) {
    return "/organizer";
  }
  return value;
}

export default async function AuthPage({
  params,
  searchParams,
}: PageProps<"/auth/[path]">) {
  await connection();
  const [{ path }, query] = await Promise.all([params, searchParams]);
  const view = allowedViews[path] ?? "SIGN_IN";

  if (!isNeonAuthConfigured()) {
    return <OrganizerAuthUnavailable />;
  }

  return <AuthPortal path={view} redirectTo={safeRedirectTo(query.redirectTo)} />;
}
