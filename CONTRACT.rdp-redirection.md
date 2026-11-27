# RDP redirection coordination

## Negotiation

The application and RDP helper retain IPC version 13. `ClientHello.server_redirection`
and `ServerHello.capabilities.server_redirection` default to false when absent.
The helper sends `RedirectRequest` only after the application opts in. The application
sends `RedirectResponse` only after the helper advertises support. Legacy peers keep
existing behavior and never receive the new message variants.

## Target and correlation

`RdpRedirectRequest` contains `session_id`, a unique `request_id`, `generation`, `hop`,
`host`, `port` and optional `connect_hosts` (default empty). One helper owns one Connect; its generation is 1. Hops increase
from 1 through 8 within that connection, including redirection after activation.
The host and port identify the target's TLS and certificate trust identity, never
the local relay. IDs contain 1–128 bytes, host contains 1–253 bytes without whitespace
or control characters, and port is nonzero. Hosts are DNS names or IPv4/IPv6 literals,
never URL authorities. `connect_hosts` has at most 16 entries with the same host
validation; it lists ordered network targets independently of the TLS identity.
An empty list means connect to `host`. Requests outside these bounds fail.

`RedirectResponse` echoes the complete request. Only equality with the outstanding
request permits consumption of a response. A stale response cannot complete or
replace a current request. The application additionally correlates requests and
asynchronous work with its live helper instance and session start token.

## Preparation

The helper waits for preparation before connecting to a redirected target.
`RdpRedirectResponse::Ready { relay: None }` permits the original direct route;
`Ready { relay: Some(endpoint) }` supplies a newly prepared target-specific relay.
A connection originally using a relay rejects a response without a relay. Relay
validation requires a loopback address, nonzero port and 32-byte authentication token.
`Rejected { message }`, timeout after 120 seconds, cancellation or closed IPC ends
preparation without connecting. No failure falls back to direct TCP.

The application preserves the original proxy and SSH jump configuration when preparing
the target route, trying `connect_hosts` in order when supplied. DNS resolution follows that route's existing policy. The helper
retains protocol Session ID and Routing Token handling; these are not UI state.

## Certificates and lifecycle

After successful preparation, the helper changes its accepted certificate endpoint
to the actual target. System trust validation remains enabled. Additional certificate
requests use that target's host and port, and the application applies the original
certificate policy to a separate target trust record. Certificate replies are matched
to the live pending request; closing or superseding a session invalidates stale work.
No trust record is written for a relay endpoint or under the original host's name.

Disconnect, cancellation and helper shutdown release pending preparation. The
application owns relay lifetime and releases old or failed routes. Relay tokens and
server reconnect cookies are never logged or persisted in saved connections.

## Component rollout

A new application with an old helper retains the old behavior and cannot claim
coordinated redirection support. A new helper with an old application does not send
redirect coordination messages; it keeps the prior certificate endpoint restriction
and rejects relay redirection. Both new peers negotiate coordination explicitly.
The protocol pin alone does not update an installed helper. Ship the component through
the independent repository's signed release pipeline; users update components manually.
