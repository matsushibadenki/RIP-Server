#!/bin/sh
set -eu
gs "$@" 1>&2
exec tar -C /output -cf - .
