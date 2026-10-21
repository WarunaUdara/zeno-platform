import { connection } from "next/server";
import { AttendeeDemo } from "@/components/overview/attendee-demo";

export default async function DemoOverviewPage() {
  await connection();
  return <AttendeeDemo />;
}
