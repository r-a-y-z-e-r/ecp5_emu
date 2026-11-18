// Cycle-level UART host. No peeking at the CPU bus or replacing firmware work.
#include "Vtop.h"
#include "verilated.h"
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <string>
int main(int argc, char **argv) {
    if (argc != 4) return 2;
    const uint64_t cycles = std::strtoull(argv[1], nullptr, 10);
    std::ifstream in(argv[2], std::ios::binary);
    const std::string input((std::istreambuf_iterator<char>(in)), {});
    FILE *out = std::fopen(argv[3], "wb");
    if (!in || !out) return 2;
    Verilated::commandArgs(argc, argv);
    Vtop top;
    top.clk_25mhz = 0; top.btn = 1; top.ftdi_txd = 1; top.eval();
    bool previous = true, receiving = false;
    uint64_t start = 0; unsigned data = 0, bytes = 0;
    for (uint64_t c = 0; c < cycles; c++) {
        // The first start bit is present at cycle zero, with no inter-byte gap.
        uint64_t frame = c / 2170, bit = c % 2170 / 217;
        top.ftdi_txd = frame >= input.size() || bit == 9 ? 1 :
                       bit == 0 ? 0 : (uint8_t(input[frame]) >> (bit - 1)) & 1;
        top.clk_25mhz = 0; top.eval();
        top.clk_25mhz = 1; top.eval();
        bool tx = top.ftdi_rxd;
        if (!receiving && previous && !tx) {
            receiving = true; start = c; data = 0;
        } else if (receiving) {
            uint64_t age = c - start;
            for (unsigned b = 0; b < 8; b++)
                if (age == 325 + b * 217) data |= unsigned(tx) << b;
            if (age == 2061) {
                if (!tx) { std::fprintf(stderr, "bad UART stop at %llu\n", (unsigned long long)c); return 1; }
                std::fputc(data, out); bytes++; receiving = false;
            }
        }
        previous = tx;
    }
    top.final(); std::fclose(out);
    std::printf("RTL: %llu cycles, %u UART bytes\n", (unsigned long long)cycles, bytes);
    return 0;
}
