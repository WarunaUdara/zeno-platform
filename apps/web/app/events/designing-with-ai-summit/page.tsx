import { redirect } from "next/navigation";
import { DEMO_EVENT_ID } from "@/lib/demo-event";

export default function DemoEventAlias() {
  redirect(`/events/${DEMO_EVENT_ID}`);
}
