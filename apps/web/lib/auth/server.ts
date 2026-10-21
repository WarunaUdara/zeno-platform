import "server-only";
import { createNeonAuth } from "@neondatabase/auth/next/server";

const minimumCookieSecretLength = 32;

export function isNeonAuthConfigured() {
  const baseUrl = process.env.NEON_AUTH_BASE_URL?.trim();
  const cookieSecret = process.env.NEON_AUTH_COOKIE_SECRET;
  if (!baseUrl || !cookieSecret || cookieSecret.length < minimumCookieSecretLength) return false;

  try {
    const url = new URL(baseUrl);
    return (
      (url.protocol === "https:" || (url.protocol === "http:" && isLoopback(url.hostname))) &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash
    );
  } catch {
    return false;
  }
}

function isLoopback(hostname: string) {
  return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "[::1]";
}

export function getNeonAuth() {
  if (!isNeonAuthConfigured()) {
    throw new Error("Neon Auth is not configured for this environment");
  }

  return createNeonAuth({
    baseUrl: process.env.NEON_AUTH_BASE_URL!.trim(),
    cookies: { secret: process.env.NEON_AUTH_COOKIE_SECRET! },
    logLevel: process.env.NODE_ENV === "production" ? "error" : "warn",
  });
}
