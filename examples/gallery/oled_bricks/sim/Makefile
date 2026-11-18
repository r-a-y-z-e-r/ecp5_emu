# Use relative source paths and one compiler recipe: GNU Make's generated
# Verilator rules do not handle this checkout's spaces. No parallel compiler jobs.
CXX = c++
verilator_root = $(shell verilator -V | sed -n 's/^ *VERILATOR_ROOT *= *//p' | head -1)
.PHONY: all
all: obj/Vtop game_obj/Vgame_tb
obj/Vtop: $(wildcard obj/*.cpp) uart_tb.cpp playtest.h wall_playtest.h serve_playtest.h
	$(CXX) -std=c++17 -O2 -pthread -Iobj -I"$(verilator_root)/include" -I"$(verilator_root)/include/vltstd" \
	  obj/*.cpp uart_tb.cpp "$(verilator_root)/include/verilated.cpp" \
	  "$(verilator_root)/include/verilated_threads.cpp" -lz -o obj/Vtop
game_obj/Vgame_tb: $(wildcard game_obj/*.cpp) game_tb.cpp
	$(CXX) -std=c++17 -O2 -pthread -Igame_obj -I"$(verilator_root)/include" -I"$(verilator_root)/include/vltstd" \
	  game_obj/*.cpp game_tb.cpp "$(verilator_root)/include/verilated.cpp" \
	  "$(verilator_root)/include/verilated_threads.cpp" -o game_obj/Vgame_tb
