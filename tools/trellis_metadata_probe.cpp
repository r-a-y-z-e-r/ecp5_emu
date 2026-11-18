// Research-only parser probe. Synthetic commands are never sent to a device.
#include "Bitstream.hpp"
#include "Chip.hpp"
#include "ChipConfig.hpp"
#include "Database.hpp"
#include <iomanip>
#include <stdexcept>

static void append(Trellis::Bitstream &b, Trellis::BitstreamCommand op, uint32_t value)
{
    b.data.push_back(uint8_t(op));
    b.data.insert(b.data.end(), {0, 0, 0});
    for (int shift = 24; shift >= 0; shift -= 8)
        b.data.push_back(uint8_t(value >> shift));
}

int main(int argc, char **argv)
{
    try {
        if (argc != 3) throw std::runtime_error("usage: trellis_metadata_probe DATABASE BIT");
        Trellis::load_database(argv[1]);
        auto bit = Trellis::Bitstream::read_bit_py(argv[2]);
        append(bit, Trellis::BitstreamCommand::LSC_PROG_CNTRL0, 0x11223344);
        append(bit, Trellis::BitstreamCommand::LSC_PROG_CNTRL1, 0x55667788);
        append(bit, Trellis::BitstreamCommand::LSC_PROG_SED_CRC, 0x89abcdef);
        append(bit, Trellis::BitstreamCommand::ISC_PROGRAM_USERCODE, 0x13579bdf);
        auto parsed = bit.deserialise_chip(boost::optional<uint32_t>());
        auto text = Trellis::ChipConfig::from_chip(parsed).to_string();
        auto restored = Trellis::ChipConfig::from_string(text).to_chip();
        if (parsed.ctrl0 != 0x11223344 || parsed.ctrl1 != 0x55667788 ||
            parsed.sed != 0x89abcdef || parsed.usercode != 0x13579bdf)
            throw std::runtime_error("native parser did not retain synthetic sentinels");
        std::cout << "field,native_Chip,after_text_config\n" << std::hex << std::setfill('0');
        auto row = [](const char *n, uint32_t a, uint32_t b) {
            std::cout << n << ",0x" << std::setw(8) << a << ",0x" << std::setw(8) << b << '\n';
        };
        row("ctrl0", parsed.ctrl0, restored.ctrl0);
        row("ctrl1", parsed.ctrl1, restored.ctrl1);
        row("sed_reference", parsed.sed, restored.sed);
        row("usercode", parsed.usercode, restored.usercode);
    } catch (const std::exception &e) {
        std::cerr << e.what() << '\n';
        return 1;
    }
}
