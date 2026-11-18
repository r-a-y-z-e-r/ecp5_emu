// Research helper: invoke native BEL constructors without building full routing.
// Input: constructor row col type index bank (whitespace separated).
// Output: TSV port records. It verifies interfaces, not placement or behaviour.
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
            throw std::runtime_error("usage: trellis_resource_probe DATABASE");
        Dl_info library;
        if (!dladdr(reinterpret_cast<const void *>(&Trellis::load_database), &library))
            throw std::runtime_error("cannot identify loaded Trellis library");
        std::cerr << "libtrellis_path=" << library.dli_fname << '\n';
        Trellis::load_database(argv[1]);
        Trellis::Chip chip("LFE5U-85F");
        Trellis::RoutingGraph graph(chip);
        std::string constructor, type;
        int row, col, index, bank;
        std::string line, extra;
        while (std::getline(std::cin, line)) {
            std::istringstream request(line);
            if (!(request >> constructor >> row >> col >> type >> index >> bank) || (request >> extra))
                throw std::runtime_error("malformed probe request");
            if (row < 0 || row > chip.get_max_row() || col < 0 || col > chip.get_max_col())
                throw std::runtime_error("requested resource outside device");
            if (constructor == "ioclk")
                Trellis::Ecp5Bels::add_ioclk_bel(graph, type, col, row, index, bank);
            else if (constructor == "misc")
                Trellis::Ecp5Bels::add_misc(graph, type, col, row);
            else if (constructor == "pll")
                Trellis::Ecp5Bels::add_pll(graph, type, col, row);
            else if (constructor == "hard") {
                if (type == "EBR")
                    Trellis::Ecp5Bels::add_bram(graph, col, row, index);
                else if (type == "MULT18")
                    Trellis::Ecp5Bels::add_mult18(graph, col, row, index);
                else if (type == "ALU54")
                    Trellis::Ecp5Bels::add_alu54b(graph, col, row, index);
                else
                    throw std::runtime_error("unknown hard resource: " + type);
            }
            else
                throw std::runtime_error("unknown constructor: " + constructor);
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
    } catch (const std::exception &e) {
        std::cerr << e.what() << '\n';
        return 1;
    }
}
