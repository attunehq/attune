#!/bin/bash

# Exit on error
set -e

# Check if ATTUNE_GPG_KEY_ID is set
if [[ -z "$ATTUNE_GPG_KEY_ID" ]]; then
    echo "Error: ATTUNE_GPG_KEY_ID environment variable is not set"
    exit 1
fi

# Check if fixtures directory exists
if [[ ! -d "fixtures" ]]; then
    echo "Error: fixtures directory does not exist"
    exit 1
fi

cargo run --bin attune -- apt repo create testing-concurrent || true

# Find all .deb files in ~/scratch
deb_files=("fixtures"/*.deb)

# Check if any .deb files exist
if [[ ! -e "${deb_files[0]}" ]]; then
    echo "No .deb files found in fixtures"
    exit 1
fi

echo "Found ${#deb_files[@]} .deb files in fixtures"
echo "Adding packages concurrently..."

# Array to store background process PIDs
pids=()

# Launch concurrent jobs
for deb_file in "${deb_files[@]}"; do
    echo "Starting: $(basename "$deb_file")"
    cargo run --bin attune -- apt pkg add -r testing-concurrent -d testing -c testing -k "$ATTUNE_GPG_KEY_ID" "$deb_file" &
    pids+=($!)
done

echo "Launched ${#pids[@]} concurrent processes"
echo "Waiting for completion..."

# Wait for all background processes and collect results
failed=0
for i in "${!pids[@]}"; do
    pid=${pids[$i]}
    deb_file=${deb_files[$i]}

    if wait $pid; then
        echo "✓ Success: $(basename "$deb_file")"
    else
        echo "✗ Failed: $(basename "$deb_file")"
        ((failed++))
    fi
done

echo "Completed: $((${#pids[@]} - failed)) succeeded, $failed failed"

if [[ $failed -gt 0 ]]; then
    exit 1
fi
