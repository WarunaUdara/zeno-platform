# Technology choices and tradeoffs

Selections are dated and revisited only for documented product, maintenance, or deployment evidence. A benchmark is meaningful only for an equivalent workload and architecture.

## Rust or Go for the API

| Choice | Strength | Cost / risk |
| --- | --- | --- |
| Rust + Axum/Tokio | Memory and type safety; async systems foundation; strong fit for concurrency-focused reliability work | More concepts and compile feedback to absorb; smaller commercial SDK ecosystem and some integration friction; build times and ownership complexity can slow a small team |
| Go + `net/http`/Chi | Fast onboarding, simple deployment, broad cloud tooling, robust race detector and mature service tooling | Garbage collector and lower-level modeling tradeoffs; safety depends on coding discipline; easier to mistake ease of writing for correct transactional semantics |

**Decision:** Rust for the API and worker, to make concurrency and failure behavior explicit. This does not establish a performance advantage for ticket checkout: database contention, payment latency, network time, and admission control dominate many workloads. Keep the code understandable, profile memory and p99 under a fixed load profile, and avoid unsafe code. Do not create extra services in Go just to show both languages. Reconsider Go if the Rust learning/runtime cost prevents shipping the first reliable organizer journey; reconsider another language only with measured evidence.

Axum is a routing/request library that composes with Tower middleware. Use typed extractors, consistent error mapping, bounded body/timeout limits, tracing, and cancellation-aware calls. Keep domain rules independent of HTTP/Tokio.

## Next.js or TanStack Start

| Choice | Strength | Cost / risk |
| --- | --- | --- |
| Next.js App Router | Stable, widely used React framework; strong SSR/metadata ecosystem for public event pages; large ecosystem and community; self-hostable in a Node container | Runtime/cache behavior is framework-specific; multi-instance ISR/tag coordination needs deliberate shared cache setup; can grow client bundles if server/client boundaries are careless |
| TanStack Start | Type-safe routing, loaders/search, and server boundaries; portable Vite/Rsbuild output; attractive client-heavy control-plane ergonomics | Current official docs identify it as Release Candidate. RC means APIs may settle and production sharp edges remain; smaller ecosystem than Next.js today |

**Decision:** Next.js for the event web and organizer console, with public pages server-rendered and cacheable, while the Rust API remains the source of truth. Self-host through the official Node/container path; do not depend on Vercel-only APIs. Configure cache handlers/tag invalidation before horizontally scaling dynamic page caching. Place data access behind generated API clients. Keep scanner software native/mobile via Expo rather than trying to turn a website into a scanner.

The React skill's high-impact guardrails apply: parallelize independent server fetches, avoid waterfalls, keep expensive client code behind dynamic imports, minimize serialized server data, and avoid shared mutable module state. Apply each optimization after the page/data boundary is clear.

## API, database, packages

- Rust/Axum owns external and mobile API behavior; OpenAPI 3.1 and Problem Details describe it. REST versioning is easier to inspect and generate for mobile/third-party integrations than exposing framework-specific server functions.
- PostgreSQL first for transactional inventory, events/orders, and measured search. Use migrations with expand/contract deployment. Add Redis only for measured cache/rate-limit or ephemeral data; never use a distributed cache as sole ticket authority.
- Next.js/React/TypeScript for web; Expo/React Native for mobile. Do not require a proprietary build/deploy service for the application to work.
- Monorepo: Cargo workspace for Rust and Bun workspaces/Turborepo for TypeScript tasks, with independently built OCI images. Avoid duplicating generated API types or running workspace-wide build work for unrelated changes.
- OpenTelemetry instrumentation plus an OTLP collector enables interchangeable telemetry destinations. Vendor dashboard exports may remain provider-specific.

## Deliberately not chosen yet

Dedicated search cluster, message broker, event sourcing, graph database, multi-region writes, self-built identity, payments settlement, service mesh, WASM tenant code, and database sharding. Introduce only from demand, quantified scale, or real regulatory requirement. A modular monolith does not prevent extracting a context after contracts and ownership are proven.
