# Local TLS materials

Development certificate authority and localhost fixtures for the local proxy.
`just local` / `just stack up` call `hostname` and generate a leaf certificate
under `infra/local/generated/<instance>/proxy/certs/`. Caddy mounts that directory
and serves `server.pem` + `server-key.pem`. The certificate covers the detected
hostname, `localhost`, `*.localhost`, and loopback IPs. The CA stays unchanged
between starts, so coworkers only need to trust it once; Tailscale is optional.

Standalone `bun run dev` in `apps/web` uses this same CA and certificate
generator. Vite serves HTTPS directly and prints the hostname URL. Its leaf
certificate is generated in a temporary directory, loaded into memory, and
removed from disk; restarting Vite does not require trusting a new CA.

These keys are not secrets. Anyone with the repo can mint a matching cert;
do not reuse them outside local development.

## Use a CA generated on this machine

`ca-key.pem` is public, so a machine that trusts `ca.pem` system-wide accepts
certificates anyone can mint. To trust a CA whose key never leaves the machine,
generate one outside the checkout, put `ca.pem` and `ca-key.pem` in a directory
and set `MACRO_LOCAL_CA_DIR` to it before `just run_local` (or `issue-host.sh`).
The proxy's leaf certificate is then signed by that CA, and `curl --cacert`,
Vite's `NODE_EXTRA_CA_CERTS` and health probes use its `ca.pem`. Trust that
`ca.pem` instead of the checked-in one, as below. The CA must allow the names
in the leaf: `localhost`, the hostname, `127.0.0.1` and `::1`. Such a leaf has
no `*.localhost`: with a name-constrained CA, rustls (the desktop app) rejects
the chain over it.

## Trust the CA (once per machine)

Trust the CA on the machine running the browser:

```bash
# macOS
sudo security add-trusted-cert -d -r trustRoot \
  -k /Library/Keychains/System.keychain infra/local/certs/ca.pem

# Linux (Debian/Ubuntu)
sudo cp infra/local/certs/ca.pem /usr/local/share/ca-certificates/macro-local-ca.crt
sudo update-ca-certificates
```

Firefox may use its own certificate store: Settings → Privacy & Security →
Certificates → View Certificates → Authorities → Import `ca.pem`, then enable
trust for identifying websites. Restart the browser after changing trust.

`curl` against the proxy can pass `--cacert infra/local/certs/ca.pem` instead.

## Issue a leaf manually

```bash
bash infra/local/certs/issue-host.sh /tmp/macro-certs "$(hostname)"
```

This preserves the CA. The generated directory contains the certificate and key.

## Regenerate the CA and localhost fixtures

This replaces the CA; everyone who trusted the previous CA must import it again.
Normal stack startup does not run this command.

```bash
bash infra/local/certs/generate.sh
```
