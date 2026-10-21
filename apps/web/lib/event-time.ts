export function createEventClock(timeZone: string) {
  let resolvedTimeZone = timeZone;
  let usesUtcFallback = false;

  try {
    new Intl.DateTimeFormat("en", { timeZone }).format(0);
  } catch {
    resolvedTimeZone = "UTC";
    usesUtcFallback = true;
  }

  return {
    label: usesUtcFallback ? "UTC" : timeZone,
    usesUtcFallback,
    format(date: Date | string, options: Intl.DateTimeFormatOptions) {
      return new Intl.DateTimeFormat("en", {
        ...options,
        timeZone: resolvedTimeZone,
      }).format(date instanceof Date ? date : new Date(date));
    },
  };
}
