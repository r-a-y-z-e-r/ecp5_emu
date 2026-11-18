// Research helper: resolve wire names using the installed libtrellis.
// Input: tile_row tile_col database_wire_name. Output: canonical row col name.
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
            throw std::runtime_error("usage: trellis_wire_probe DATABASE");
        Dl_info library;
        if (!dladdr(reinterpret_cast<const void *>(&Trellis::load_database), &library))
            throw std::runtime_error("cannot identify libtrellis");
        std::cerr << "libtrellis_path=" << library.dli_fname << '\n';
        Trellis::load_database(argv[1]);
        Trellis::Chip chip("LFE5U-85F");
        Trellis::RoutingGraph graph(chip);
        std::string line, name, extra;
        int row, col;
        while (std::getline(std::cin, line)) {
            std::istringstream input(line);
            if (!(input >> row >> col >> name) || (input >> extra))
                throw std::runtime_error("invalid request");
            if (row < 0 || row > chip.get_max_row() || col < 0 || col > chip.get_max_col())
                throw std::runtime_error("tile outside device");
            const auto result = graph.globalise_net(row, col, name);
            if (result.id < 0)
                std::cout << "invalid\n";
            else
                std::cout << result.loc.y << '\t' << result.loc.x << '\t'
                          << graph.to_str(result.id) << '\n';
        }
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
