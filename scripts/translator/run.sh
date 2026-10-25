#!/bin/bash

# This is the translation tool intended for machine-translating strings.
#
# ###########################
# # Adding new translations #
# ###########################
# 
# If you want to add a new language, follow these steps:
#
# 1. Include the language in `wlx-common/src/locale.rs`:
#   - add your entry in `enum Language` 
#   - add your entry in Language::code() function
#   - add your language code in Language::add_codes() function
# 
# 2. Create a new template file
#   - Add your language code in `scripts/translator/templates/<lang code>.json`
#     (you can copy one of nearby files and adapt them accordingly)
# 
# 3. Modify this very file
#   - Go to the bottom and add your language code entry here too
#
# 4. Run the script (it will hint you which LANG_PATHs are supported)
# 
# ############################################
# # Running machine translations (local LLM) #
# ############################################
#
# Do not run this script if you don't have at least ~16 GiB of free system memory.
# Submitting PRs that lack translated strings is okay if you don't meet the
# system requirements to run this script, or if you simply prefer not to; we are
# regularly updating the missing translation strings anyway.
#
# Base language: English (en.json)
# 
# This tool expects llama.cpp server being present (llama-server) at port 8080.
# 
# Currently we're using Gemma 4 for machine translations.
# Model URL:
# https://huggingface.co/unsloth/gemma-4-31B-it-GGUF
# Recommended quantization: Q4_K_M
# 

set -e
cd "$(dirname "$0")"

bun install

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
TEMPLATE="sv" bun main.ts
TEMPLATE="zh_CN" bun main.ts
