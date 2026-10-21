## What changed, and why

<!-- Explain the motivation. The diff already says what changed; this says why
     the change is being made and what problem it solves. -->

## How it was verified

<!-- Name the commands you ran and what they proved. "Tests pass" is weaker than
     "the new test fails on the previous commit and passes on this one". -->

## Risk

<!-- What could this break, and what would you watch after deploying? Write
     "none known" if that is genuinely the case. -->

## Checklist

- [ ] Commits are reviewable on their own and explain intent, not just the diff
- [ ] API tests pass (`cargo test --workspace`)
- [ ] Web tests, typecheck, lint, and build pass
- [ ] OpenAPI contract and generated client are in sync (`bun run api:client:check`)
- [ ] No credential, key, or token is included
- [ ] `gitleaks git --config .gitleaks.toml` is clean
- [ ] Documentation updated where behaviour changed
- [ ] No payment code was added (`orders.payment_status` stays `not_required`)
