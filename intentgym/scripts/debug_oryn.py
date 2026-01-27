import sys
import os

try:
    import oryn

    print(f"Oryn package found: {oryn.__file__}")
    from oryn import OrynClientSync, BinaryNotFoundError
except ImportError as e:
    print(f"Failed to import oryn: {e}")
    sys.exit(1)

binary_path = "/home/rohit/work/dragonscale/oryn-remote/target/debug/oryn"
if not os.path.exists(binary_path):
    print(f"Binary not found at {binary_path}")
else:
    print(f"Binary found at {binary_path}")

print("Attempting to instantiate OrynClientSync...")
try:
    client = OrynClientSync(mode="headless", binary_path=binary_path)
    print("Instance created.")
    print("Attempting connect...")
    client.connect()
    print("Connected!")

    url = "http://localhost:8765/miniwob/click-button.html"
    print(f"Navigating to {url}...")
    client.execute(f'goto "{url}"')

    print("Attempting scan...")
    scan_res = client.execute("scan")
    print(f"Scan Result:\n{scan_res}")

    print("Attempting observe 1...")
    obs = client.observe()
    print(f"Observation 1: {obs}")

    print("Attempting click...")
    # Try clicking the wrapper div since we don't know button text yet
    res = client.execute('click "wrap"')
    print(f"Click result: {res}")

    print("Attempting observe 2...")
    obs = client.observe()
    print(f"Observation 2: {obs}")

    client.close()
    print("Closed.")

except BinaryNotFoundError:
    print("Error: BinaryNotFoundError caught!")
except Exception as e:
    print(f"Error caught: {e}")
    import traceback

    traceback.print_exc()
