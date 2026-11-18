//! Refuse user pads retained by a persistent configuration port. Configuration
//! transactions themselves are outside the post-load execution model.
use super::*;

const PORTS: [&str; 3] = ["MASTER_SPI_PORT", "SLAVE_SPI_PORT", "SLAVE_PARALLEL_PORT"];

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct ConfigurationPortUse {
    pub field: String,
    pub enabled: bool,
    pub directive: Option<String>,
    pub rule: String,
}

fn retains(field: &str, pin: &crate::model::PackagePin) -> bool {
    if pin.function == "CCLK" {
        return true;
    }
    // sysCONFIG tables 4.8/6.2/6.3 define the primary pins. Reserve quad
    // lanes, secondary SPI aliases and parallel daisy-chain outputs
    // conservatively; their mode-dependent release is not executed here.
    pin.dual_function.split('/').any(|function| {
        let function = function.trim();
        match field {
            "SYSCONFIG.MASTER_SPI_PORT" => matches!(
                function,
                "MCSN"
                    | "CSSPIN"
                    | "MOSI"
                    | "MISO"
                    | "SI"
                    | "SO"
                    | "D0"
                    | "D1"
                    | "D2"
                    | "D3"
                    | "IO0"
                    | "IO1"
                    | "IO2"
                    | "IO3"
                    | "MOSI2"
                    | "MISO2"
            ),
            "SYSCONFIG.SLAVE_SPI_PORT" => {
                matches!(function, "SN" | "MOSI" | "MISO" | "SI" | "SO")
            }
            "SYSCONFIG.SLAVE_PARALLEL_PORT" => matches!(
                function,
                "D0" | "D1"
                    | "D2"
                    | "D3"
                    | "D4"
                    | "D5"
                    | "D6"
                    | "D7"
                    | "CSN"
                    | "CS1N"
                    | "WRITEN"
                    | "BUSY"
                    | "DOUT"
                    | "CSON"
            ),
            _ => false,
        }
    })
}

impl Planner {
    pub(super) fn configure_configuration_ports(&mut self) -> Result<()> {
        for name in PORTS {
            let field = format!("SYSCONFIG.{name}");
            // nextpnr writes tile enums; a hand-written .sysconfig directive
            // survives as metadata but Trellis to_chip does not apply it.
            // Inspect both, never allowing a DISABLE directive to hide CRAM.
            let directive = self
                .device
                .image()
                .service()
                .sysconfig_directives
                .get(name)
                .cloned();
            let mut enabled = match directive.as_deref() {
                None | Some("DISABLE") => false,
                Some("ENABLE") => true,
                Some(value) => {
                    bail!("unsupported {field} directive {value}; expected DISABLE or ENABLE")
                }
            };
            let tiles = self
                .device
                .model()
                .tiles()
                .filter(|(_, tile)| {
                    self.device.model().database().types[&tile.kind]
                        .enums
                        .contains_key(&field)
                })
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            let mut replicas = BTreeSet::new();
            for tile in tiles {
                let values = self.field(tile, &field)?;
                ensure!(
                    values.len() == 1 && matches!(values[0].as_str(), "DISABLE" | "ENABLE"),
                    "unresolved {field}: {values:?}"
                );
                replicas.insert(values[0] == "ENABLE");
            }
            ensure!(replicas.len() <= 1, "inconsistent {field} CRAM replicas");
            enabled |= replicas.contains(&true);
            self.report.system.configuration_ports.push(ConfigurationPortUse {
                field,
                enabled,
                directive,
                rule: if enabled {
                    "no effect on this plan: retained package pads unused; configuration transport not executed"
                } else {
                    "DISABLE: normal user IO; absent directive/field defaults to DISABLE"
                }.into(),
            });
        }
        Ok(())
    }

    pub(super) fn check_configuration_pad(&self, id: ResourceId) -> Result<()> {
        if !self
            .report
            .system
            .configuration_ports
            .iter()
            .any(|port| port.enabled)
        {
            return Ok(());
        }
        let cclk = self.device.model().resource(id).family == "USRMCLK";
        for pin in self.device.model().package_pins().filter(|pin| {
            pin.package == self.package && (pin.pio == Some(id) || (cclk && pin.function == "CCLK"))
        }) {
            for port in self
                .report
                .system
                .configuration_ports
                .iter()
                .filter(|port| port.enabled)
            {
                ensure!(
                    !retains(&port.field, pin),
                    "{}=ENABLE retains package {} ball {} ({}); user IO is not supported",
                    port.field,
                    pin.package,
                    pin.ball,
                    if cclk {
                        &pin.function
                    } else {
                        &pin.dual_function
                    }
                );
            }
        }
        Ok(())
    }

    pub(super) fn finish_configuration_ports(&self) -> Result<()> {
        // Check configuration ownership even when observed scope never reaches
        // the USRMCLK sink. Prefer a reached GPIO's specific conflict first.
        if self
            .report
            .system
            .configuration_ports
            .iter()
            .any(|port| port.enabled)
            && self
                .device
                .model()
                .resources()
                .any(|(_, resource)| resource.family == "USRMCLK")
        {
            let cclk = cclk_pad(&self.device, &self.package)?;
            if cclk.direction.is_some() {
                let port = self
                    .report
                    .system
                    .configuration_ports
                    .iter()
                    .find(|port| port.enabled)
                    .unwrap();
                bail!("{}=ENABLE retains package {} ball {} (CCLK); ownership conflicts with configured USRMCLK",
                    port.field, self.package, cclk.ball);
            }
        }
        Ok(())
    }
}
