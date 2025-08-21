#!/usr/bin/env bash

BASEDIR=$(dirname "$0")
git diff HEAD^ HEAD --quiet -- "$BASEDIR"
