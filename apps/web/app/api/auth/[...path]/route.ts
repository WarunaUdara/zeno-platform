import { getNeonAuth, isNeonAuthConfigured } from "@/lib/auth/server";

type AuthRouteContext = { params: Promise<{ path: string[] }> };
type AuthMethod = "GET" | "POST" | "PUT" | "DELETE" | "PATCH";

export const dynamic = "force-dynamic";

async function forward(method: AuthMethod, request: Request, context: AuthRouteContext) {
  if (!isNeonAuthConfigured()) {
    return Response.json({ error: "Organizer sign-in is not configured." }, { status: 503 });
  }

  return getNeonAuth().handler()[method](request, context);
}

export const GET = (request: Request, context: AuthRouteContext) => forward("GET", request, context);
export const POST = (request: Request, context: AuthRouteContext) => forward("POST", request, context);
export const PUT = (request: Request, context: AuthRouteContext) => forward("PUT", request, context);
export const DELETE = (request: Request, context: AuthRouteContext) => forward("DELETE", request, context);
export const PATCH = (request: Request, context: AuthRouteContext) => forward("PATCH", request, context);
