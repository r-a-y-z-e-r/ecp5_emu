// Research only: verify coarse/split PLC interfaces at every PLC2 location.
// No routing graph of the configured chip and no behavioral model are built.
#include "Bels.hpp"
#include "Database.hpp"
#include "RoutingGraph.hpp"
#include "Tile.hpp"
#include <algorithm>
#include <dlfcn.h>
#include <iostream>
#include <set>
#include <sstream>
#include <stdexcept>
#include <vector>

int main(int argc, char **argv)
{
    try {
        if (argc != 2) throw std::runtime_error("usage: trellis_plc_probe DATABASE");
        Dl_info library;
        if (!dladdr(reinterpret_cast<const void *>(&Trellis::load_database), &library))
            throw std::runtime_error("cannot identify loaded Trellis library");
        std::cerr << "libtrellis_path=" << library.dli_fname << '\n';
        Trellis::load_database(argv[1]);
        Trellis::Chip chip("LFE5U-85F");
        Trellis::RoutingGraph graph(chip);
        std::set<std::pair<int, int>> expected, checked;
        for (const auto &tile : chip.tiles)
            if (tile.second->info.type == "PLC2") expected.emplace(tile.second->row, tile.second->col);
        std::vector<std::string> reference;
        std::string line, extra;
        std::cout << "view\tbel\tbel_type\tz\tport\tdirection\twire_row_offset\twire_col_offset\twire\n";
        while (std::getline(std::cin, line)) {
            int row, col;
            std::istringstream input(line);
            if (!(input >> row >> col) || (input >> extra) || !expected.count({row,col}) || !checked.emplace(row,col).second)
                throw std::runtime_error("invalid or duplicate PLC2 request");
            for (int i = 0; i < 4; ++i) Trellis::CommonBels::add_lc(graph, col, row, i);
            for (int i = 0; i < 8; ++i) {
                Trellis::CommonBels::add_logic_comb(graph, col, row, i);
                Trellis::CommonBels::add_ff(graph, col, row, i);
            }
            Trellis::CommonBels::add_ramw(graph, col, row);
            auto &location = graph.tiles.at(Trellis::Location(col,row));
            std::vector<std::string> signature;
            for (const auto &entry : location.bels) {
                const auto &bel = entry.second;
                const auto type = graph.to_str(bel.type);
                for (const auto &pin : bel.pins) {
                    const auto &wire = pin.second.first;
                    if (wire.loc.x != col || wire.loc.y != row)
                        throw std::runtime_error("unexpected nonlocal constructor pin");
                    std::ostringstream record;
                    record << (type == "SLICE" ? "coarse" : "split") << '\t'
                           << graph.to_str(bel.name) << '\t' << type << '\t' << bel.z << '\t'
                           << graph.to_str(pin.first) << '\t'
                           << (pin.second.second == Trellis::PORT_IN ? "in" : "out") << '\t'
                           << wire.loc.y-row << '\t' << wire.loc.x-col << '\t' << graph.to_str(wire.id);
                    signature.push_back(record.str());
                }
            }
            std::sort(signature.begin(), signature.end());
            if (checked.size() == 1) {
                reference = signature;
                for (const auto &record : reference) std::cout << record << '\n';
            } else if (signature != reference) throw std::runtime_error("PLC interface differs by location");
            // All constructor pins are local; retain the graph's ID store but release the previous PLC.
            location.bels.clear();
            location.wires.clear();
        }
        if (checked != expected) throw std::runtime_error("requests did not cover every PLC2");
        std::cerr << "verified_plcs=" << checked.size() << '\n';
        std::cerr << "template_ports=" << reference.size() << '\n';
    } catch (const std::exception &e) {
        std::cerr << e.what() << '\n';
        return 1;
    }
}
