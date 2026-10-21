import { LandingPage } from "@/components/landing/landing-page";
import { connection } from "next/server";

export default async function Home() {
  await connection();
  return <LandingPage />;
}
