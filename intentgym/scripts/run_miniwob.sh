#!/bin/bash
set -e

DATA_DIR="$HOME/.intentgym/miniwob"
PORT=8765

echo "Setup MiniWoB++ environment..."

# Create directory
if [ ! -d "$DATA_DIR" ]; then
    echo "Creating directory $DATA_DIR"
    mkdir -p "$DATA_DIR"
fi

# Clone repository if not exists
if [ ! -d "$DATA_DIR/miniwob-plusplus" ]; then
    echo "Cloning MiniWoB++ repository..."
    git clone --depth 1 https://github.com/Farama-Foundation/miniwob-plusplus.git "$DATA_DIR/miniwob-plusplus"
else
    echo "Repository already exists."
fi

# Start server
echo "Starting HTTP server on port $PORT..."
echo "Serving from $DATA_DIR/miniwob-plusplus/html"
echo "Access tasks at http://localhost:$PORT/miniwob/"

cd "$DATA_DIR/miniwob-plusplus/miniwob/html"
python3 -m http.server $PORT
