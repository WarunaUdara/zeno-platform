import http from "k6/http";
import { check } from "k6";
import { Counter } from "k6/metrics";

const BASE_URL = (__ENV.BASE_URL || "http://127.0.0.1:18080").replace(/\/+$/, "");
if (!/^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/.test(BASE_URL)) {
  throw new Error("BASE_URL must be an HTTP loopback address; remote targets are refused.");
}

const rate = Number(__ENV.RATE || 25);
const duration = __ENV.DURATION || "30s";
if (!Number.isInteger(rate) || rate < 1 || rate > 50) {
  throw new Error("RATE must be an integer between 1 and 50 iterations per second.");
}
if (!/^([1-9]|[1-5][0-9]|60)s$/.test(duration)) {
  throw new Error("DURATION must be between 1s and 60s.");
}

const unexpectedResponses = new Counter("reservation_unexpected_responses");
const createdReservations = new Counter("reservation_created");
const inventoryRejections = new Counter("reservation_inventory_rejections");
const runId = Date.now().toString(36);
const base64UrlAlphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

function encodeIdempotencyKey(value) {
  let encoded = "";
  for (let index = 0; index < value.length; index += 3) {
    const first = value.charCodeAt(index);
    const hasSecond = index + 1 < value.length;
    const hasThird = index + 2 < value.length;
    const second = hasSecond ? value.charCodeAt(index + 1) : 0;
    const third = hasThird ? value.charCodeAt(index + 2) : 0;

    encoded += base64UrlAlphabet[first >> 2];
    encoded += base64UrlAlphabet[((first & 3) << 4) | (second >> 4)];
    if (hasSecond) encoded += base64UrlAlphabet[((second & 15) << 2) | (third >> 6)];
    if (hasThird) encoded += base64UrlAlphabet[third & 63];
  }
  return encoded;
}

http.setResponseCallback(http.expectedStatuses(200, 201, 409));

export const options = {
  summaryTrendStats: ["avg", "min", "med", "max", "p(90)", "p(95)", "p(99)"],
  scenarios: {
    hot_ticket: {
      executor: "constant-arrival-rate",
      rate,
      timeUnit: "1s",
      duration,
      preAllocatedVUs: Math.max(rate, 10),
      maxVUs: Math.min(rate * 2, 100),
    },
  },
  thresholds: {
    http_req_failed: ["rate<0.01"],
    dropped_iterations: ["count==0"],
    reservation_unexpected_responses: ["count==0"],
  },
};

export function setup() {
  const catalogResponse = http.get(`${BASE_URL}/v1/events?limit=50`);
  if (catalogResponse.status !== 200) {
    throw new Error(`Event catalog setup failed with HTTP ${catalogResponse.status}.`);
  }
  const events = catalogResponse.json().items || [];
  const event = events[0];
  if (!event?.id) throw new Error("The isolated load-test database has no published demo event.");

  const ticketResponse = http.get(`${BASE_URL}/v1/events/${encodeURIComponent(event.id)}/ticket-types`);
  if (ticketResponse.status !== 200) {
    throw new Error(`Ticket setup failed with HTTP ${ticketResponse.status}.`);
  }
  const ticketTypes = ticketResponse.json().items || [];
  const ticket = ticketTypes.find((item) => item.availability === "available");
  if (!ticket?.id) throw new Error("The isolated demo event has no available ticket type.");

  return { eventId: event.id, ticketTypeId: ticket.id };
}

export default function reserveHotTicket({ eventId, ticketTypeId }) {
  const idempotencyKey = encodeIdempotencyKey(
    `k6-loadtest-${runId}-vu-${__VU}-iter-${__ITER}-reservation`,
  );
  const response = http.post(
    `${BASE_URL}/v1/events/${encodeURIComponent(eventId)}/ticket-types/${encodeURIComponent(ticketTypeId)}/reservations`,
    JSON.stringify({ quantity: 1 }),
    {
      headers: {
        "Content-Type": "application/json",
        "Idempotency-Key": idempotencyKey,
      },
      tags: { name: "create_reservation" },
    },
  );

  let body = {};
  try {
    body = response.json();
  } catch {
    // Count malformed upstream responses as unexpected rather than masking them as transport errors.
  }

  const created = response.status === 201 && body.status === "held";
  const inventoryRejected = response.status === 409 && body.error?.code === "INVENTORY_UNAVAILABLE";
  if (created) createdReservations.add(1);
  else if (inventoryRejected) inventoryRejections.add(1);
  else unexpectedResponses.add(1);

  check(response, {
    "reservation created or rejected only after inventory is exhausted": () => created || inventoryRejected,
  });
}
