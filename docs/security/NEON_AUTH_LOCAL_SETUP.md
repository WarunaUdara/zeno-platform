# Local organizer authentication setup

The organizer routes use Neon Auth for browser sessions and request a short-lived user token on the server before calling the API. The API independently verifies that token and checks the issuer/subject against application-owned organization membership. The browser never receives the API bearer token through this integration.

## Configuration

Use a development Neon branch and synthetic identities. Do not copy production database credentials or attendee data into local development.

Configure the Next.js app (for example in an ignored `apps/web/.env.local` file) with:

```dotenv
NEON_AUTH_BASE_URL=https://<your-neon-auth-endpoint>
NEON_AUTH_COOKIE_SECRET=<random-secret-at-least-32-characters>
```

Configure the API runtime (for example in the ignored root `.env` used by Compose) with the same issuer base URL and the JWKS URL shown by the Neon Auth project:

```dotenv
NEON_AUTH_BASE_URL=https://<your-neon-auth-endpoint>
NEON_AUTH_JWKS_URL=https://<your-neon-auth-endpoint>/<provider-documented-jwks-path>
```

Generate a unique cookie secret for local use. Keep all values out of source control, build arguments, images, browser bundles and logs. The web and API values must refer to the same development identity issuer. Local HTTP is accepted only for loopback URLs; hosted endpoints must use HTTPS.

The repository's `neon.ts` declares Neon Auth and preserves the existing private preview bucket. That file alone does not configure or deploy a remote Neon project. Verify the linked project, branch, Auth settings, redirect origins, and email/SMTP behavior before applying provider-side changes. Do not run `neon deploy` against a production branch as part of local setup.

## Verification before organizer use

1. Start the local API and web app with synthetic data and verify the web build and API tests.
2. Sign in through the app's `/auth/sign-in` route and confirm the server can load `/v1/organizations`.
3. Confirm a member can access only their own organization's events; try a synthetic second issuer/subject and verify cross-tenant reads and writes are denied.
4. Sign out, expire the session, and simulate Neon Auth/JWKS unavailability. The organizer UI should not allow writes in any of these states.
5. Inspect logs and browser network traffic to confirm cookies and bearer tokens are not exposed in application logs, URLs, or client-visible API responses.

Current limitation: the installed `@neondatabase/auth` and `@neondatabase/auth-ui` versions are beta. This integration has passed local lint, typecheck, unit tests and production compilation, but has not been exercised against a configured Neon Auth issuer or browser session. Treat production organizer authentication as unverified until the steps above pass against an isolated project.
