-- Lexer categories: case-insensitive keywords, -- comments, strings, characters, bit strings, based numbers, attributes.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity sample is
  generic (WIDTH : positive := 8);
  port (clk, rst : in std_logic; data : out std_logic_vector(WIDTH-1 downto 0));
end entity;
architecture rtl of sample is
  signal count : unsigned(WIDTH-1 downto 0) := (others => '0');
  constant quoted : string := "a ""quoted"" string";
  constant mask : bit_vector(7 downto 0) := X"AF";
  constant binary : bit_vector(3 downto 0) := B"10_01";
  constant sized : bit_vector(7 downto 0) := 8X"FF";
  constant based : integer := 16#FF#E+1;
  constant fraction : real := 1.25E-3;
  signal \extended name\ : std_logic;
begin
  PROCESS(clk)
  begin
    if clk'event and clk = '1' then
      if rst = '1' then count <= (others => '0');
      else count <= count + 1;
      end if;
    end if;
  end process;
  data <= std_logic_vector(count);
  assert WIDTH > 0 report quoted severity note;
end architecture;
