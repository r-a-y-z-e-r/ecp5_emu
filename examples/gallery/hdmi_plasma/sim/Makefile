# Relative paths plus one quoted compiler invocation handle checkout spaces.
# Invoked only with make -j 1. No generated Verilator makefile is used.
CXX = c++
verilator_root = $(shell verilator -V | sed -n 's/^ *VERILATOR_ROOT *= *//p' | head -1)
.PHONY: all
all: obj/Vtop
obj/Vtop: $(wildcard obj/*.cpp) uart_tb.cpp
	$(CXX) -std=c++17 -O2 -pthread -Iobj -I"$(verilator_root)/include" -I"$(verilator_root)/include/vltstd" \
	  obj/*.cpp uart_tb.cpp "$(verilator_root)/include/verilated.cpp" \
	  "$(verilator_root)/include/verilated_threads.cpp" -lz -o obj/Vtop
