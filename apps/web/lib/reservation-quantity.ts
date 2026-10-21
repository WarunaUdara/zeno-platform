export const MAX_RESERVATION_QUANTITY = 100;

export function parseReservationQuantity(value: string): number | null {
  if (!/^\d+$/.test(value)) return null;
  const quantity = Number(value);
  return Number.isSafeInteger(quantity) && quantity >= 1 && quantity <= MAX_RESERVATION_QUANTITY
    ? quantity
    : null;
}
