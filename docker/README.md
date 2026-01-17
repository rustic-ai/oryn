# Oryn Docker Images

Container variants optimized for different use cases:

| Image | Base | Browser | Size | Use Case |
|-------|------|---------|------|----------|
| `oryn-h:headless` | chromedp/headless-shell | Chrome Headless Shell | ~324MB | Smallest, recommended |
| `oryn-e:debian` | Debian Bookworm | WPE WebKit + COG | ~437MB | WPE WebKit option |
| `oryn-e:alpine` | Alpine 3.21 | WPE WebKit + COG | ~483MB | Alpine-based |
| `oryn-h:ubuntu` | Ubuntu (latest) | Chromium | ~536MB | Full Chromium |

**Recommendation:** Use `oryn-h:headless` for smallest footprint and Chrome compatibility, or `oryn-e:debian` for WPE WebKit.

**Notes:**
- Debian Bookworm uses LLVM 15 (~23MB), Alpine 3.21 uses LLVM 19 (~154MB)
- Alpine 3.21 required for WPE/COG packages (not available in 3.23+)

## Building

```bash
# Build all images
docker compose build

# Or build individually
docker build -f docker/Dockerfile.oryn-h.headless -t oryn-h:headless .  # Smallest
docker build -f docker/Dockerfile.oryn-e.debian -t oryn-e:debian .
docker build -f docker/Dockerfile.oryn-e -t oryn-e:alpine .
docker build -f docker/Dockerfile.oryn-h -t oryn-h:ubuntu .
```

## Running

### Interactive Mode

```bash
# Embedded backend (WPE)
docker run -it --rm \
  -e COG_PLATFORM_NAME=headless \
  -v /dev/shm:/dev/shm \
  --security-opt seccomp=unconfined \
  oryn-e:latest \
  oryn-e --url https://example.com

# Headless backend (Chromium)
docker run -it --rm \
  --shm-size=2gb \
  -v /dev/shm:/dev/shm \
  --security-opt seccomp=unconfined \
  oryn-h:latest \
  oryn-h --url https://example.com
```

### Using Docker Compose

```bash
# Start embedded backend
docker compose run --rm oryn-e oryn-e --url https://example.com

# Start headless backend
docker compose run --rm oryn-h oryn-h --url https://example.com
```

## Environment Variables

### oryn-e (WPE)
- `COG_PLATFORM_NAME=headless` - Use headless rendering (required in containers)
- `XDG_RUNTIME_DIR` - Runtime directory for Wayland (`/run/user/1000` on Alpine, `/run/user/1001` on Debian)

### oryn-h (Chromium)
- `CHROME_BIN` - Path to Chromium binary (default: `/usr/bin/chromium-browser`)
- `CHROMIUM_FLAGS` - Additional Chromium flags

## Security Considerations

All containers run as non-root user `oryn` for security:
- Alpine: UID 1000
- Debian/Ubuntu: UID 1001 (avoids conflict with existing users)

Browser automation requires some elevated permissions:
- `--security-opt seccomp=unconfined` - Required for browser sandboxing
- `--shm-size=2gb` - Recommended for Chromium to prevent crashes
- `--cap-add SYS_ADMIN` - May be needed for some operations

## Testing

### Full Test Suite

Build and test all images:

```bash
cd docker

# Test all images (build + verify)
./test-images.sh all

# Test specific image
./test-images.sh alpine
./test-images.sh debian
./test-images.sh ubuntu

# Include navigation tests (slower, requires network)
./test-images.sh --nav all
```

### Smoke Test

Quick verification of a pre-built image:

```bash
# Test Alpine WPE image
./smoke-test.sh oryn-e:alpine

# Test Ubuntu Chromium image
./smoke-test.sh oryn-h:ubuntu

# Include navigation test
./smoke-test.sh oryn-e:alpine --nav
```

### What's Tested

| Test | Description |
|------|-------------|
| Binary exists | oryn-e or oryn-h binary is present |
| Binary runs | --help executes successfully |
| Browser deps | WPEWebDriver/COG or Chromium installed |
| Security | Running as non-root user |
| Environment | XDG_RUNTIME_DIR, CA certs present |
| Navigation | (optional) Load https://example.com |

## Known Limitations

### Headless Shell/oryn-h:headless
- Uses Chrome Headless Shell (not full Chromium) - optimized for automation
- Based on [chromedp/headless-shell](https://github.com/chromedp/docker-headless-shell)
- Requires `--shm-size=2g` to prevent crashes

### Alpine/oryn-e
- Screenshot support is limited in headless mode (no weston)
- Some sites may render differently than Chromium

### Ubuntu/oryn-h
- Larger image size due to full Chromium dependencies
- Requires more memory (recommend 2GB+ shm)
