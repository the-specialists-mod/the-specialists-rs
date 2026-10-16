#!/bin/bash
HL=${HL:-$(cd "$(dirname "$0")/../../../../.." && pwd)}  # the directory holding this repo, ts30-decomp, ts_original and ts21
# usage: dis.sh so|mp|cl|mp21|cl21 <ghidra-addr hex> <len hex/dec>
case $1 in
  so) B=$HL/ts_original/dlls/ts_i386.so; off=$((0x10000));;
  mp) B=$HL/ts_original/dlls/mp.dll; off=0;;
  cl) B=$(ls $HL/ts_original/cl_dlls/client.dll);off=0;;
  mp21) B=$HL/ts21/dlls/mp.dll; off=0;;
  cl21) B=$HL/ts21/cl_dlls/client.dll; off=0;;
esac
s=$(( $2 - off )); e=$(( s + $3 ))
objdump -d --no-show-raw-insn -M intel --start-address=$s --stop-address=$e $B | tail -n +7
