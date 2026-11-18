// Runtime image reader: native CRAM/EBR and service registers, no text round-trip.
// Input bytes arrive on stdin; no hardware transport is opened.
#include "Bitstream.hpp"
#include "Chip.hpp"
#include "ChipConfig.hpp"
#include "Database.hpp"
#include <iostream>
#include <sstream>
#include <stdexcept>

static void hex_bytes(std::ostream &out, const std::string &s)
{
    static const char hex[] = "0123456789abcdef";
    out << '"';
    for (unsigned char c : s) out << hex[c >> 4] << hex[c & 15];
    out << '"';
}

int main(int argc, char **argv)
{
    // Keep stdin/stdout buffered independently of C stdio, including JSON writes.
    std::ios::sync_with_stdio(false);
    std::cin.tie(nullptr);
    try {
        if (argc != 3) throw std::runtime_error("usage: trellis_image_reader DATABASE bit|config < SOURCE");
        const std::string mode(argv[2]);
        if (mode != "bit" && mode != "config") throw std::runtime_error("unknown source kind");
        Trellis::load_database(argv[1]);
        std::string input;
        char buffer[64 * 1024];
        while (std::cin) {
            std::cin.read(buffer, sizeof buffer);
            input.append(buffer, static_cast<std::size_t>(std::cin.gcount()));
        }
        auto load = [&]() {
            if (mode == "bit") {
                std::istringstream stream(input);
                return Trellis::Bitstream::read_bit(stream).deserialise_chip(boost::optional<uint32_t>());
            }
            return Trellis::ChipConfig::from_string(input).to_chip();
        };
        auto chip = load();
        if (chip.info.name != "LFE5U-85F" || chip.cram.bits() % 8 != 0)
            throw std::runtime_error("unsupported chip or CRAM layout");
        auto &out = std::cout;
        out << "{\"schema_version\":1,\"image_reader_schema\":1,\"device\":\"LFE5U-85F\",\"frames\":"
            << chip.cram.frames() << ",\"bits_per_frame\":" << chip.cram.bits()
            << ",\"encoding\":\"frame_byte_lsb0\",\"origin\":\""
            << (mode == "bit" ? "bitstream" : "text_config") << "\",\"cram_hex\":\"";
        static const char hex[] = "0123456789abcdef";
        for (int f = 0; f < chip.cram.frames(); ++f)
            for (int b = 0; b < chip.cram.bits(); b += 8) {
                unsigned byte = 0;
                for (int i = 0; i < 8; ++i) byte |= unsigned(chip.cram.bit(f, b + i)) << i;
                out << hex[byte >> 4] << hex[byte & 15];
            }
        out << "\",\"bram_data\":{";
        bool first = true;
        for (const auto &image : chip.bram_data) {
            if (!first) out << ',';
            first = false;
            out << '"' << image.first << "\":[";
            bool first_word = true;
            for (auto word : image.second) {
                if (!first_word) out << ',';
                first_word = false;
                out << word;
            }
            out << ']';
        }
        out << "},\"native_registers\":";
        if (mode == "bit")
            out << "{\"ctrl0\":" << chip.ctrl0 << ",\"ctrl1\":" << chip.ctrl1
                << ",\"sed_reference\":" << chip.sed << ",\"usercode\":" << chip.usercode << '}';
        else out << "null"; // Chip constructor defaults are not text-file evidence.
        out << ",\"parser_defaults_may_apply\":" << (mode == "bit" ? "true" : "false")
            << ",\"native_metadata_hex\":[";
        first = true;
        for (const auto &s : chip.metadata) {
            if (!first) out << ',';
            first = false;
            hex_bytes(out, s);
        }
        out << "]}\n";
        out.flush();
        if (!out) throw std::runtime_error("failed to write decoded image");
    } catch (const std::exception &e) {
        std::cerr << e.what() << '\n';
        return 1;
    }
}
