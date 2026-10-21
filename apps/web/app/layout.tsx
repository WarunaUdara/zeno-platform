import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Zeno — events that run smoothly",
  description: "Discover events and build reliable event experiences with Zeno.",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return <html lang="en"><body>{children}</body></html>;
}
