# Settings copy — private HTTPS setup

Proposed English copy for the HTTPS guide in Settings. This is a content asset for the planning decision, not an installed feature or a tested deployment recipe. Exact proxy screens depend on the selected proxy/version; the guide must link the corresponding instructions before shipping.

---

## Set up HTTPS for your private server

To connect Spotify on a separate server, first open scrobbl.rs through HTTPS. Your server can stay private: you do not need to open ports on your router.

**Already have a domain?** Use a name under your domain. **Don't have one?** You can use a free DuckDNS name instead. Choose the guide that applies to you.

### I don't have a domain — use DuckDNS

1. Open [DuckDNS](https://www.duckdns.org/), sign in and register an available name, such as `your-name.duckdns.org`.
2. Configure a reverse proxy on your server to handle HTTPS for that exact name and forward requests to scrobbl.rs. Choose a proxy or certificate tool that supports the DuckDNS DNS challenge.
3. Request a Let's Encrypt certificate using **DNS challenge / DNS-01**, select **DuckDNS** as the provider and enter the token from your DuckDNS account in the certificate tool's credential settings. Nginx Proxy Manager is one graphical option that supports this; other compatible tools can be used. Enable automatic renewal and keep the certificate tool's configuration and data persistent. Do not paste the token into this page, a callback URL, a screenshot or a support message.
4. In your local DNS server, make that name resolve to the private IP address of the reverse proxy. In AdGuard Home, add a DNS rewrite with the exact hostname as the domain and the proxy's LAN IP as the answer. Enter only the hostname here, without `https://`, a path or a port. The devices opening scrobbl.rs must use that DNS configuration.
5. Configure scrobbl.rs to use the same HTTPS address as its external application URL. Include a port if your proxy uses one other than 443. Open that address and sign in before connecting Spotify.

Your DuckDNS name supplies the hostname. The certificate tool verifies it through DNS, so scrobbl.rs does not need to be reachable from the public internet. This guide does not require setting DuckDNS to your router's public IP or forwarding router ports.

### I already have a domain

1. Choose a name you control, such as `scrobbl.example.com`.
2. Configure a reverse proxy on your server to handle HTTPS for that exact name and forward requests to scrobbl.rs.
3. Request a certificate using **DNS challenge / DNS-01** with the provider hosting your domain's DNS records, which may differ from the company where you bought the domain. Follow the certificate tool's instructions for that provider; its credentials will differ from DuckDNS. Prefer a supported DNS API so issuance and renewal can run automatically. Owning a domain does not mean every DNS provider supports the same automation.
4. Make that name resolve to the reverse proxy's private IP in your local DNS. With AdGuard Home, use a DNS rewrite. Ensure the devices opening scrobbl.rs use that DNS configuration.
5. Set the same HTTPS address as scrobbl.rs's external application URL, including any custom port, then open it and sign in.

Keep DNS credentials in the certificate tool's credential settings and retain its configuration for renewal. A certificate issued by your own provider can also be used if it is trusted by your browsers and its renewal is maintained.

### Connect Spotify

1. Open scrobbl.rs at the HTTPS address you configured. The browser should recognize the certificate without a security warning.
2. In Settings, create or configure your Spotify app using the existing Spotify guide. Copy the **exact redirect URI displayed by scrobbl.rs** into the Spotify Developer Dashboard. It must use the same HTTPS hostname, path and any custom port.
3. Save the Client ID and Client Secret, choose **Connect Spotify**, then approve access on Spotify in the same browser/profile.
4. Spotify returns you to scrobbl.rs. Check **Connected** and **Last successful sync**. Your server keeps collecting after you close the browser. Reconnecting later uses this same browser flow.

### If setup does not work

- **Name does not reach the server:** check the local DNS rewrite and the DNS configuration used by the device/browser.
- **Browser certificate warning:** check the certificate hostname, expiry and certificate chain. Fix the configuration before continuing.
- **HTTPS port unavailable:** the TrueNAS management interface or another service may already use it. Give the proxy an appropriate LAN address or port. If using a custom port, include it consistently in the application URL and Spotify redirect URI.
- **Spotify rejects the redirect:** copy the exact HTTPS redirect URI from Settings again and check the registered value.
- **Authorization cancelled or expired:** return to Settings and start a fresh connection attempt. Your listening archive is kept.
- **Certificate renewal failed:** check the DNS provider credentials and renewal status in your certificate tool.

The HTTPS and Spotify setup instructions must remain separate from the certificate tool's secret inputs. Settings explains how to configure the infrastructure; it does not promise automatic installation of a proxy, DNS changes or certificate issuance.
