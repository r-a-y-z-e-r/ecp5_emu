// Research helper: IO BEL interfaces only; no full routing or behaviour.
// Input: row col letter simple (simple is 0 for IOLOGIC, 1 for SIOLOGIC).
#include "Bels.hpp"
#include "Database.hpp"
#include "RoutingGraph.hpp"
#include <dlfcn.h>
#include <iostream>
#include <sstream>
#include <stdexcept>

int main(int argc, char **argv)
{
    try {
        if (argc != 2)
            throw std::runtime_error("usage: trellis_io_probe DATABASE");
        Dl_info library;
        if (!dladdr(reinterpret_cast<const void *>(&Trellis::load_database), &library))
            throw std::runtime_error("cannot identify loaded Trellis library");
        std::cerr << "libtrellis_path=" << library.dli_fname << '\n';
        Trellis::load_database(argv[1]);
        Trellis::Chip chip("LFE5U-85F");
        Trellis::RoutingGraph graph(chip);
        std::string line, extra;
        while (std::getline(std::cin, line)) {
            std::istringstream request(line);
            int row, col, index, simple;
            if (!(request >> row >> col >> index >> simple) || (request >> extra))
                throw std::runtime_error("malformed IO probe request");
            if (row < 0 || row > chip.get_max_row() || col < 0 || col > chip.get_max_col() ||
                index < 0 || index > (simple ? 1 : 3) || (simple != 0 && simple != 1))
                throw std::runtime_error("invalid IO probe location");
            Trellis::Ecp5Bels::add_iologic(graph, col, row, index, simple != 0);
            Trellis::Ecp5Bels::add_pio(graph, col, row, index);
        }
        std::cout << "row\tcol\tbel\tbel_type\tport\tdirection\twire_row\twire_col\twire\n";
        for (const auto &location : graph.tiles)
            for (const auto &entry : location.second.bels) {
                const auto &bel = entry.second;
                for (const auto &pin : bel.pins) {
                    const auto &wire = pin.second.first;
                    const auto direction = pin.second.second;
                    std::cout << bel.loc.y << '\t' << bel.loc.x << '\t'
                              << graph.to_str(bel.name) << '\t' << graph.to_str(bel.type) << '\t'
                              << graph.to_str(pin.first) << '\t'
                              << (direction == Trellis::PORT_IN ? "in" : direction == Trellis::PORT_OUT ? "out" : "inout")
                              << '\t' << wire.loc.y << '\t' << wire.loc.x << '\t'
                              << graph.to_str(wire.id) << '\n';
                }
            }
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
