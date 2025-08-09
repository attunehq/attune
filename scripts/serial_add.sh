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

cargo run --bin attune -- apt repo create testing-serial || true

# Find all .deb files in fixtures
deb_files=("fixtures"/*.deb)

# Check if any .deb files exist
if [[ ! -e "${deb_files[0]}" ]]; then
    echo "No .deb files found in fixtures"
    exit 1
fi

echo "Found ${#deb_files[@]} .deb files in fixtures"
echo "Adding packages serially..."

# Process files one by one
failed=0
for deb_file in "${deb_files[@]}"; do
    echo "Processing: $(basename "$deb_file")"

    if cargo run --bin attune -- apt pkg add -r testing-serial -d jammy -c testing -k "$ATTUNE_GPG_KEY_ID" "$deb_file"; then
        echo "✓ Success: $(basename "$deb_file")"
    else
        echo "✗ Failed: $(basename "$deb_file")"
        ((failed++))
    fi
done

echo "Completed: $((${#deb_files[@]} - failed)) succeeded, $failed failed"

if [[ $failed -gt 0 ]]; then
    exit 1
fi
