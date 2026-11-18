library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity top is
  port (clk_25mhz : in std_logic;
        led : out std_logic_vector(7 downto 0));
end entity;

architecture rtl of top is
  signal count : unsigned(7 downto 0) := (others => '0');
begin
  process (clk_25mhz)
  begin
    if rising_edge(clk_25mhz) then
      count <= count + 1;
    end if;
  end process;
  led <= std_logic_vector(count);
end architecture;
