const localDateTimePattern = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/;

function localPartsAt(timestamp: number, timeZone: string) {
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone,
    calendar: "gregory",
    numberingSystem: "latn",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  }).formatToParts(timestamp);
  const value = Object.fromEntries(parts.map((part) => [part.type, part.value]));
  return {
    year: Number(value.year),
    month: Number(value.month),
    day: Number(value.day),
    hour: Number(value.hour),
    minute: Number(value.minute),
    second: Number(value.second),
  };
}

function sameParts(
  left: ReturnType<typeof localPartsAt>,
  right: ReturnType<typeof localPartsAt>,
) {
  return (
    left.year === right.year &&
    left.month === right.month &&
    left.day === right.day &&
    left.hour === right.hour &&
    left.minute === right.minute &&
    left.second === right.second
  );
}

export function localDateTimeToIso(value: string, timeZone: string) {
  const match = localDateTimePattern.exec(value);
  if (!match) return null;

  const requested = {
    year: Number(match[1]),
    month: Number(match[2]),
    day: Number(match[3]),
    hour: Number(match[4]),
    minute: Number(match[5]),
    second: Number(match[6] ?? "0"),
  };
  const localAsUtc = Date.UTC(
    requested.year,
    requested.month - 1,
    requested.day,
    requested.hour,
    requested.minute,
    requested.second,
  );
  if (!Number.isFinite(localAsUtc) || !sameParts(requested, localPartsAt(localAsUtc, "UTC"))) return null;

  try {
    const offsets = new Set<number>();
    for (const hours of [-36, -24, -12, 0, 12, 24, 36]) {
      const sample = localAsUtc + hours * 60 * 60 * 1000;
      const local = localPartsAt(sample, timeZone);
      const localAsUtcAtSample = Date.UTC(
        local.year,
        local.month - 1,
        local.day,
        local.hour,
        local.minute,
        local.second,
      );
      offsets.add(localAsUtcAtSample - sample);
    }

    const matches = [...offsets]
      .map((offset) => localAsUtc - offset)
      .filter((candidate) => sameParts(requested, localPartsAt(candidate, timeZone)))
      .sort((left, right) => left - right);

    // For a repeated daylight-saving hour, choose the earlier instant. A skipped hour has no match.
    return matches.length ? new Date(matches[0]).toISOString() : null;
  } catch {
    return null;
  }
}

export function isoToLocalDateTime(value: string | null, timeZone: string) {
  if (!value) return "";
  const timestamp = new Date(value);
  if (!Number.isFinite(timestamp.valueOf())) return "";
  try {
    const parts = localPartsAt(timestamp.valueOf(), timeZone);
    const pad = (part: number) => String(part).padStart(2, "0");
    return `${parts.year}-${pad(parts.month)}-${pad(parts.day)}T${pad(parts.hour)}:${pad(parts.minute)}`;
  } catch {
    return "";
  }
}

export function convertLocalDateTimesToTimeZone(
  values: { startsAt: string; endsAt: string },
  fromTimeZone: string,
  toTimeZone: string,
) {
  const startsAt = localDateTimeToIso(values.startsAt, fromTimeZone);
  if (!startsAt) return null;

  const endsAt = values.endsAt ? localDateTimeToIso(values.endsAt, fromTimeZone) : null;
  if (values.endsAt && !endsAt) return null;

  const convertedStart = isoToLocalDateTime(startsAt, toTimeZone);
  const convertedEnd = endsAt ? isoToLocalDateTime(endsAt, toTimeZone) : "";
  if (!convertedStart || (endsAt && !convertedEnd)) return null;

  return { startsAt: convertedStart, endsAt: convertedEnd };
}
