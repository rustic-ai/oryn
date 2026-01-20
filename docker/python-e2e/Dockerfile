# Dockerfile for Python E2E tests
# Contains all oryn binaries + Python + oryn-python client
#
# Build context should be the project root:
#   docker build -t oryn-python-e2e -f docker/python-e2e/Dockerfile .

FROM python:3.11-slim

# Install system dependencies for running oryn backends
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    chromium \
    chromium-driver \
    fonts-liberation \
    libasound2 \
    libatk-bridge2.0-0 \
    libatk1.0-0 \
    libcups2 \
    libdbus-1-3 \
    libdrm2 \
    libgbm1 \
    libgtk-3-0 \
    libnspr4 \
    libnss3 \
    libxcomposite1 \
    libxdamage1 \
    libxfixes3 \
    libxkbcommon0 \
    libxrandr2 \
    xdg-utils \
    && rm -rf /var/lib/apt/lists/*

# Set up working directory
WORKDIR /app

# Copy pre-built oryn binaries from host
# These should be built with: cargo build --release
COPY target/release/oryn /usr/local/bin/oryn
RUN chmod +x /usr/local/bin/oryn

# Verify oryn is executable
RUN oryn --version || echo "oryn binary installed"

# Install poetry
RUN pip install --no-cache-dir poetry

# Copy oryn-python library
COPY oryn-python/pyproject.toml oryn-python/poetry.lock* /app/oryn-python/
COPY oryn-python/src /app/oryn-python/src
COPY oryn-python/tests /app/oryn-python/tests
COPY oryn-python/README.md /app/oryn-python/

# Install oryn-python with poetry
WORKDIR /app/oryn-python
RUN poetry config virtualenvs.create false && \
    poetry install --no-interaction --no-ansi

# Copy test harness scripts (.oil files)
COPY test-harness/scripts/ /app/test-harness/scripts/

# Set environment variables
ENV PYTHONUNBUFFERED=1
ENV ORYN_MODE=headless
ENV TEST_HARNESS_URL=http://host.docker.internal:3000

# For Linux hosts without host.docker.internal
# The run script will override this with --add-host or --network host

# Default command runs the .oil script tests
WORKDIR /app/oryn-python
CMD ["poetry", "run", "pytest", "tests/e2e/test_oil_scripts.py", "-v", "--tb=short"]
