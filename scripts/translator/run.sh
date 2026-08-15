#!/bin/bash

# Do not run this script if you don't have at least 32 GiB of system memory.
# Submitting PRs that lack translated strings is okay if you don't meet the
# system requirements to run this script, or if you simply prefer not to; we are
# regularly updating the missing translation strings anyway.
#
# Base language: English (en.json)
# 
# This tool expects llama.cpp server being present (llama-server) at port 8080.

set -e
cd "$(dirname "$0")"

bun install

# model URL:
# https://huggingface.co/unsloth/gemma-4-31B-it-GGUF
# you can specify your own if you want to.
if [ -z "${MODEL}" ]; then
	export MODEL="gemma-4-31B-it-Q4_K_M"
fi

if [ -z "${LLAMA_BASE_URL}" ]; then
	export LLAMA_BASE_URL="http://127.0.0.1:8080"
fi

TEMPLATE="en" bun main.ts
TEMPLATE="pl" bun main.ts
TEMPLATE="de" bun main.ts
TEMPLATE="ja" bun main.ts
TEMPLATE="es" bun main.ts
TEMPLATE="it" bun main.ts
TEMPLATE="zh_CN" bun main.ts
