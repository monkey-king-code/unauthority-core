# Tor Binary for Linux

**This directory should contain:**
- `tor` - Tor daemon for Linux (x86_64)

## How to obtain:
1. Install via package manager:
   ```bash
   # Ubuntu/Debian
   sudo apt-get install tor
   cp /usr/bin/tor ./tor
   
   # Arch Linux
   sudo pacman -S tor
   cp /usr/bin/tor ./tor
   
   # Fedora
   sudo dnf install tor
   cp /usr/bin/tor ./tor
   ```

2. Download from Tor Project:
   - https://www.torproject.org/download/tor/
   - Extract `tor` binary

3. Make executable:
   ```bash
   chmod +x tor
   ```

## Requirements:
- Linux x86_64
- glibc 2.17 or later
