// Read-only traffic generator used to make the "Zeno API" dashboard show a
// real spike, so the documented drills can be reproduced instead of relying on
// a folder of stale screenshots.
//
// It drives the web tier rather than the API directly, because that is the path
// a browser takes: the web tier renders the event list on the server and calls
// the API, so the API sees genuine /v1/events traffic. It only reads, so it
// never consumes inventory or creates holds.
//
//   k6 run infra/observability/spike-traffic.js
//   RATE=40 DURATION=120s k6 run infra/observability/spike-traffic.js
//
// The defaults are deliberately modest. This runs against a laptop; a bigger
// number is not a better result, and the timing floor on one machine is noise.
import http from "k6/http";
import { check } from "k6";

const BASE_URL = (__ENV.BASE_URL || "http://127.0.0.1:3000").replace(/\/+$/, "");
if (!/^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/.test(BASE_URL)) {
  throw new Error("BASE_URL must be an HTTP loopback address; remote targets are refused.");
}

const rate = Number(__ENV.RATE || 25);
const duration = __ENV.DURATION || "90s";
if (!Number.isInteger(rate) || rate < 1 || rate > 200) {
  throw new Error("RATE must be an integer between 1 and 200 requests per second.");
}

export const options = {
  scenarios: {
    browse: {
      // A constant arrival rate shows up as a clean step on the dashboard,
      // which is the shape the screenshot is trying to demonstrate.
      executor: "constant-arrival-rate",
      rate,
      timeUnit: "1s",
      duration,
      preAllocatedVUs: Math.min(rate, 20),
      maxVUs: Math.min(rate * 2, 100),
    },
  },
  thresholds: {
    // The point of the exercise: load goes up, errors do not. If the harness
    // itself starts failing connections, that is worth knowing about, not
    // hiding, so the run fails loudly instead.
    http_req_failed: ["rate<0.01"],
  },
};

export default function () {
  const res = http.get(`${BASE_URL}/events`);
  check(res, { "event list served": (r) => r.status === 200 });
}
