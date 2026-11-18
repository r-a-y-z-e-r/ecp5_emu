"""Portable integrity checks for the checked-in architecture research data.

These test the exported data contract, including unresolved information. Native
constructor verification is a separate opt-in generation step.
"""
from collections import Counter, defaultdict
import csv
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "docs/inventory/LFE5U-85F"


def read_csv(name):
    with (DATA / name).open() as stream:
        return list(csv.DictReader(stream))


class ResourceCatalogueTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.resources = read_csv("resource_instances.csv")
        cls.by_id = {r["resource_id"]: r for r in cls.resources}
        cls.bindings = read_csv("site_bindings.csv")
        cls.ports = read_csv("resource_ports.csv")
        cls.fields = read_csv("config_field_bindings.csv")
        cls.bits = read_csv("config_field_bits.csv")
        cls.report = json.loads((DATA / "binding_report.json").read_text())

    def test_raw_records_are_preserved_with_independent_identity(self):
        columns = ("tile", "site_name", "vendor_pos_row", "vendor_pos_col")
        original = read_csv("site_references.csv")
        self.assertEqual(Counter(tuple(r[c] for c in columns) for r in original),
                         Counter(tuple(r[c] for c in columns) for r in self.bindings))
        self.assertEqual(len(original), len({(r["tile"], r["site_index"]) for r in self.bindings}))

    def test_identifiers_and_targets_are_unique_and_present(self):
        self.assertEqual(len(self.resources), len(self.by_id))
        for r in self.resources:
            if r["parent_id"]: self.assertIn(r["parent_id"], self.by_id)
        for r in self.bindings:
            if r["resource_id"]: self.assertIn(r["resource_id"], self.by_id)
        self.assertEqual(dict(sorted(Counter(r["relation"] for r in self.bindings).items())),
                         self.report["site_binding_counts"])

    def test_pio_set_and_every_package_pin_match_independent_metadata(self):
        inventory = json.loads((DATA / "device_inventory.json").read_text())
        actual = {(int(r["fabric_row"]), int(r["fabric_col"]), r["name"][-1])
                  for r in self.resources if r["family"] == "PIO"}
        metadata = {(r["row"], r["col"], r["pio"]) for r in inventory["pio_metadata"]}
        self.assertEqual(actual, metadata)
        for r in read_csv("package_pins.csv"):
            self.assertIn((int(r["fabric_row"]), int(r["fabric_col"]), r["pio"]), actual)

    def test_dqs_fragments_share_one_instance_with_distinct_fields(self):
        groups = defaultdict(list)
        for r in self.bindings:
            if r["label_family"] == "DQS": groups[r["resource_id"]].append(r)
        self.assertEqual(len(groups), 14)
        for key, refs in groups.items():
            self.assertEqual(self.by_id[key]["family"], "DQSBUFM")
            self.assertEqual(len(refs), 4)
            self.assertEqual(len({r["tile"] for r in refs}), 4)
            fields = [f for f in self.fields if f["resource_id"] == key]
            self.assertEqual({f["tile"] for f in fields}, {r["tile"] for r in refs})
            self.assertEqual(sum(f["field"] == "DQS.MODE" for f in fields), 3)

    def test_constructor_omissions_remain_explicit(self):
        top = [r for r in self.resources if r["family"] == "DLLDELD" and r["fabric_row"] == "0"]
        self.assertEqual({r["fabric_col"] for r in top}, {"66", "67", "68", "69"})
        self.assertTrue(all(r["native_bel"] == "False" for r in top))
        centre = [r for r in self.resources if r["family"] == "DCC_CENTRE"]
        self.assertEqual(len(centre), 4)
        self.assertTrue(all(r["native_bel"] == "False" for r in centre))

    def test_uncertain_and_unsupported_records_are_not_promoted(self):
        sync = [r for r in self.bindings if r["label_family"] == "ECLKSYNC"]
        self.assertEqual(len(sync), 8)
        self.assertTrue(all(r["relation"] == "instance_candidate" for r in sync))
        for family in ("CIBTEST", "CLKTEST", "BREFTEST", "PVTTEST", "START", "BCINRD", "BCLVDSO"):
            refs = [r for r in self.bindings if r["label_family"] == family]
            self.assertTrue(refs)
            self.assertTrue(all(r["relation"] == "unresolved" for r in refs))

    def test_ports_keep_direction_location_and_global_oscillator_alias(self):
        self.assertEqual(len(self.ports), len({(r["resource_id"], r["port"]) for r in self.ports}))
        for r in self.ports:
            self.assertIn(r["resource_id"], self.by_id)
            self.assertIn(r["direction"], ("in", "out", "inout"))
        oscillator = next(r for r in self.ports if r["resource_id"].endswith("/OSCG") and r["port"] == "OSC")
        self.assertEqual((oscillator["wire_row"], oscillator["wire_col"], oscillator["wire"]), ("0", "0", "G_JOSC_OSC"))
        self.assertEqual(self.by_id[oscillator["resource_id"]]["fabric_row"], "94")

    def test_cram_provenance_and_cross_resource_bit_overlap_are_retained(self):
        tiles = {r["tile"]: r for r in read_csv("tile_instances.csv")}
        fields = {r["field_id"]: r for r in self.fields}
        self.assertEqual(len(fields), len(self.fields))
        counts, bit_owners = Counter(), defaultdict(list)
        for b in self.bits:
            f, t = fields[b["field_id"]], tiles[b["tile"]]
            self.assertEqual(f["resource_id"], b["resource_id"])
            self.assertEqual(f["tile"], b["tile"])
            self.assertLess(int(b["local_frame"]), int(t["frame_count"]))
            self.assertLess(int(b["local_bit"]), int(t["bit_count"]))
            self.assertEqual(int(b["cram_frame"]), int(t["start_frame"]) + int(b["local_frame"]))
            self.assertEqual(int(b["cram_bit"]), int(t["start_bit"]) + int(b["local_bit"]))
            counts[b["field_id"]] += 1
            bit_owners[b["cram_frame"], b["cram_bit"]].append(f)
        self.assertTrue(all(counts[f["field_id"]] == int(f["bit_count"]) for f in self.fields))
        overlap = bit_owners["508", "1135"]
        self.assertEqual({f["field"] for f in overlap}, {"OSC.DIV", "CCLK.MODE"})
        self.assertEqual(len({f["resource_id"] for f in overlap}), 2)

    def test_fixed_clock_bypasses_remain_open_interpretation_issues(self):
        issues = read_csv("routing_interpretation_issues.csv")
        self.assertEqual(len(issues), 8)
        fields = {f["field_id"]: f for f in self.fields}
        for r in issues:
            self.assertEqual(self.by_id[r["resource_id"]]["family"], "ECLKSYNCB")
            self.assertEqual(fields[r["mode_field_id"]]["resource_id"], r["resource_id"])

    def test_all_ebr_settings_follow_the_documented_two_or_three_tile_span(self):
        by_resource = defaultdict(list)
        for f in self.fields: by_resource[f["resource_id"]].append(f)
        ebrs = [r for r in self.resources if r["family"] == "EBR"]
        self.assertEqual(len(ebrs), 208)
        for r in ebrs:
            fields = by_resource[r["resource_id"]]
            self.assertTrue(fields)
            owner_tiles = {f["tile"].split(":")[0] for f in fields}
            row, col = int(r["fabric_row"]), int(r["fabric_col"])
            width = 2 if r["name"] == "EBR0" else 3
            self.assertEqual(owner_tiles, {f"MIB_R{row}C{col+i}" for i in range(width)})
            self.assertEqual(sum(f["field"] == r["name"] + ".WID" for f in fields), 2 if r["name"] == "EBR3" else 1)

    def test_wid_keeps_nine_observed_bits_across_split_word_fragments(self):
        groups = defaultdict(list)
        for row in read_csv("config_word_layout.csv"):
            if row["field"].endswith(".WID"): groups[row["resource_id"]].append(row)
        self.assertEqual(len(groups), 208)
        for resource, layout in groups.items():
            observations = [b for b in layout if b["mapping_status"] == "observed"]
            self.assertEqual(sorted(int(b["logical_bit_lsb0"]) for b in observations), list(range(9)))
            for bit in layout:
                self.assertEqual(bit["width"], "9")
                if bit["mapping_status"] == "no_bits_in_fragment":
                    self.assertEqual(json.loads(bit["cram_constraints_for_one"]), [])

    def test_tail_ebr_mode_alias_is_distinguished_by_the_whole_block(self):
        target = "LFE5U-85F/R22C105/EBR0"
        aliases = [a for a in read_csv("enum_fragment_aliases.csv")
                   if a["resource_id"] == target and a["field_id"].endswith("/enum/EBR0.MODE")]
        self.assertTrue(any(set(json.loads(a["options"])) == {"DP16KD", "PDPW16KD"} for a in aliases))
        group = next(g for g in read_csv("config_field_groups.csv") if g["resource_id"] == target and g["field"] == "EBR0.MODE")
        self.assertEqual(group["fragment_count"], "2")
        self.assertEqual(json.loads(group["combined_encoding_aliases"]), [])

    def test_dsp_slices_group_two_multipliers_and_one_alu_without_new_storage(self):
        children = defaultdict(list)
        for r in self.resources: children[r["parent_id"]].append(r)
        slices = [r for r in self.resources if r["family"] == "DSP_SLICE"]
        self.assertEqual(len(slices), 78)
        for r in slices:
            self.assertEqual(r["native_bel"], "False")
            self.assertEqual(self.by_id[r["parent_id"]]["family"], "DSP_BLOCK")
            self.assertEqual(Counter(c["family"] for c in children[r["resource_id"]]), {"MULT18": 2, "ALU54": 1})

    def test_dsp_common_control_bit_keeps_both_primitive_views(self):
        shared = next(b for b in read_csv("cross_resource_config_bits.csv") if (b["cram_frame"], b["cram_bit"]) == ("383", "122"))
        fields = json.loads(shared["field_ids"])
        self.assertEqual({f.rsplit("/", 1)[-1] for f in fields}, {"ALU54_3.REG_INPUTC1_CLK", "MULT18_1.REG_INPUTC_CLK"})
        self.assertEqual(len(json.loads(shared["resource_ids"])), 2)
        group = next(g for g in read_csv("config_field_groups.csv")
                     if g["resource_id"] == "LFE5U-85F/R10C105/MULT18_0" and g["field"] == "MULT18_0.MODE")
        self.assertEqual(set(json.loads(group["defaults"])), {"NONE", "MULT18X18D"})
        self.assertEqual(group["defaults_differ"], "True")

    def test_service_evidence_does_not_invent_unknown_interfaces(self):
        evidence = json.loads((DATA / "unmodelled_resource_evidence.json").read_text())["families"]
        cib = evidence["CIBTEST"]
        self.assertEqual(len(cib["observed_wire_names"]), 64)
        self.assertTrue(cib["fixed_connection_descriptors"])
        for name in ("CLKTEST", "BREFTEST", "PVTTEST", "START", "BCINRD", "BCLVDSO"):
            self.assertGreater(evidence[name]["site_reference_records"], 0)
            self.assertEqual(evidence[name]["observed_wire_names"], [])
        self.assertEqual(evidence["POR"]["tile_instances"], 1)
        for name in ("PRADD9", "PRADD18", "MULT9", "ALU24"):
            self.assertTrue(evidence[name]["observed_wire_names"])
            self.assertEqual(evidence[name]["setting_descriptors"], [])


if __name__ == "__main__":
    unittest.main()
