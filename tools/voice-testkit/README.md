# voice-testkit

Local fake Discord voice gateway for manual testing of the client's voice
transport and microphone path. It is a development tool and is not part of the
application build.

## What it tests

- Voice Gateway WebSocket handshake (Identify, Ready, Select Protocol, heartbeats)
- UDP IP discovery
- RTP transport encryption
- Opus frames sent by the client's microphone

DAVE (E2EE/MLS) is **disabled** (`dave_protocol_version = 0`). This testkit
checks transport and audio sending, not the real DAVE handshake. For DAVE, use a
real Discord voice channel.

## Requirements

- A debug build of the client. The endpoint override is compiled out of release
  builds on purpose, so a voice token cannot be redirected in production.
- Linux with `/etc/ssl/certs/ca-certificates.crt` (the helper script merges the
  test certificate with the system CA bundle).

## Usage

```bash
scripts/voice-testkit.sh            # start the testkit and print the env vars
scripts/voice-testkit.sh --with-app # start the testkit and the debug client
```

Then join a voice channel through the client. Discord still issues a real voice
token; the client ignores the real voice endpoint and connects to the local
testkit instead. The testkit does not validate the token, so any Discord session
works.

The script prints the two variables used by the debug client:

```bash
DISCORD_VOICE_ENDPOINT_OVERRIDE=localhost:<port>
SSL_CERT_FILE=/tmp/opencode/oto-testkit-ca-bundle.pem
```

`SSL_CERT_FILE` replaces the system trust store in rustls, so the script writes
a bundle containing both the testkit certificate and the system CA bundle.

## Logs and stats

The testkit writes to `/tmp/opencode/voice-testkit.log` by default:

```text
STATS udp_packets=... discovery_responses=... gateway_records=...
```

- `gateway_records` grows after the client connects to the voice gateway.
- `discovery_responses` grows after UDP IP discovery.
- `udp_packets` grows while the microphone sends RTP.

Stop the testkit with `Ctrl+C`. Restart the client without the environment
variables afterwards to return to normal Discord voice.
