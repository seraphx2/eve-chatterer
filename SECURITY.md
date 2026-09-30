# Security Policy

EVE Chatterer is a young, single-maintainer project, so things here are quick
and informal (see [CONTRIBUTING.md](CONTRIBUTING.md)). Security reports get one
exception to that: please don't open a public issue.

## Supported versions

Only the latest release gets fixes; there's no backport policy given the
CalVer / single-maintainer setup. If you're on an older version, update first
and confirm the issue still reproduces.

## Reporting a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/seraphx2/eve-chatterer/security/advisories/new).
It's visible only to the maintainer until there's a fix.

No formal SLA (solo maintainer), but expect an initial response within a few
days. Reporters are credited in the published advisory unless they'd rather
stay anonymous.

## Scope

This covers EVE Chatterer's own code (`core/`, `app/`, `cli/`, `tools/`) and
its release workflows, including the self-updater and its signed releases. A
vulnerability in an upstream dependency should usually go to that project
directly; if it affects EVE Chatterer specifically (how a dependency is used
here, not the dependency in the abstract), a report here is still welcome.
