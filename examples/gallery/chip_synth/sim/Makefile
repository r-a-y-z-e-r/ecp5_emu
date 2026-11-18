# Compile copied relative sources; quote include paths for checkouts with spaces.
CXX = c++
verilator_root = $(shell verilator -V | sed -n 's/^ *VERILATOR_ROOT *= *//p' | head -1)
.PHONY: all
all: obj/Vtop
obj/Vtop: $(wildcard obj/*.cpp) tb.cpp
	$(CXX) -std=c++17 -O2 -pthread -Iobj -I"$(verilator_root)/include" -I"$(verilator_root)/include/vltstd" \
	  obj/*.cpp tb.cpp "$(verilator_root)/include/verilated.cpp" \
	  "$(verilator_root)/include/verilated_threads.cpp" -lz -o obj/Vtop
