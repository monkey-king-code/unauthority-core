# LOS Peer Directory

**A single static HTML file** to browse active validators on the Unauthority (LOS) network.

The `.onion` address for this page is listed in the main GitHub README — new users simply open it, see active validators, and copy-paste addresses into their wallet app.

## How It Works

```
New user opens .onion from README
          │
          ▼
┌─────────────────────┐
│    index.html        │  ← Single file, no backend
│    (JavaScript)      │
│                      │
│  fetch() directly    │
│  to 4 bootstrap      │
│  validator .onion    │
└─────────┬───────────┘
          │  GET /directory/api/active
          ▼
┌─────────────────────┐
│ Validator Nodes     │  ← Already have embedded endpoint
│  V1 .onion:3030     │
│  V2 .onion:3031     │
│  V3 .onion:3032     │
│  V4 .onion:3033     │
└─────────────────────┘
```

- **No server/backend** — just 1 `index.html` file
- JavaScript `fetch()` directly to validator nodes
- CORS is set to `allow_any_origin()` in los-node
- Auto-refreshes every 90 seconds

## Files

```
tools/peer-directory/
├── index.html    ← The only file needed
└── README.md
```

## Deploy to .onion

### 1. Install Tor

```bash
# macOS
brew install tor

# Ubuntu/Debian
sudo apt install tor
```

### 2. Configure Hidden Service

Add to `/etc/tor/torrc` (or `/opt/homebrew/etc/tor/torrc`):

```
HiddenServiceDir /var/lib/tor/los-peer-directory/
HiddenServicePort 80 127.0.0.1:8080
```

### 3. Restart Tor & Get .onion Address

```bash
sudo systemctl restart tor
# or: brew services restart tor

cat /var/lib/tor/los-peer-directory/hostname
# Output: abc123xyz.onion
```

### 4. Serve the HTML File

Pick one:

```bash
# Python (simplest, already installed)
cd tools/peer-directory
python3 -m http.server 8080

# Or nginx (production)
# Copy index.html to /var/www/los-directory/
# Config nginx listen 127.0.0.1:8080, root /var/www/los-directory/
```

### 5. Add to README

```markdown
## Peer Directory
Open in Tor Browser: http://abc123xyz.onion
```

## Requirements

- **Users must use Tor Browser** — `.onion` addresses are only accessible via Tor
- **Validators must be online** — JavaScript fetches from validators; if all are offline the page still loads but shows no data
- **CORS headers** — Validator los-node already enables `allow_any_origin()` on all routes

## Security

- No tracking, no analytics
- No data stored on the server (there is no server)
- All data is fetched client-side and exists only in browser memory
- The page cannot be manipulated — it only displays data directly from validators

## Notes

- **Free forever** — `.onion` requires no paid domain or hosting
- **Deploy anywhere** — Home computer, Raspberry Pi, cheap VPS
- **Anyone can deploy** — No special keys required
- **Redundancy** — Multiple people can deploy their own peer directory
