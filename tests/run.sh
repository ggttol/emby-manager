#!/bin/sh
# tests/run.sh — 跑全部 tests/test_*.py
# 用法: ./tests/run.sh   或   sh tests/run.sh
cd "$(dirname "$0")/.." && python3 -m unittest discover tests -v
