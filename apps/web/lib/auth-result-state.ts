type AuthReadResult<T> = {
  data?: T | null;
  error?: unknown | null;
};

function isUnauthenticatedError(error: unknown) {
  return typeof error === "object"
    && error !== null
    && "status" in error
    && error.status === 401;
}

export function classifySessionLookup<TUser>(result: AuthReadResult<{ user?: TUser | null }>) {
  if (result.error !== undefined && result.error !== null) {
    return { state: isUnauthenticatedError(result.error) ? "signed-out" as const : "unavailable" as const };
  }
  const user = result.data?.user;
  return user ? { state: "authenticated" as const, user } : { state: "signed-out" as const };
}

export function classifyTokenLookup(result: AuthReadResult<{ token?: unknown | null }>) {
  if (result.error !== undefined && result.error !== null) {
    return { state: isUnauthenticatedError(result.error) ? "signed-out" as const : "unavailable" as const };
  }
  const token = result.data?.token;
  return typeof token === "string" && token.length > 0
    ? { state: "ready" as const, token }
    : { state: "unavailable" as const };
}
