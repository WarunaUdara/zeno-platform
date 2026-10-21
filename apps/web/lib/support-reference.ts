const REQUEST_ID_PATTERN = /^req_[0-9A-HJKMNP-TV-Z]{26}$/;

export function supportReference(
  headerValue: string | null,
  bodyValue?: unknown,
): string | null {
  const candidate = headerValue ?? bodyValue;
  return typeof candidate === "string" && REQUEST_ID_PATTERN.test(candidate)
    ? candidate
    : null;
}
