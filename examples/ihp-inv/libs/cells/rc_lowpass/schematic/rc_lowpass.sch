<Qucs Schematic 25.2.0>
<Properties>
  <View=0,0,800,600,1,0,0>
  <Grid=10,10,1>
  <DataSet=rc_lowpass.dat>
  <DataDisplay=rc_lowpass.dpl>
  <OpenDisplay=0>
  <Script=rc_lowpass.m>
  <RunScript=0>
  <showFrame=0>
  <FrameText0=Title>
  <FrameText1=Drawn By:>
  <FrameText2=Date:>
  <FrameText3=Revision:>
</Properties>
<Symbol>
</Symbol>
<Components>
  <Vac V1 1 150 250 18 -26 0 1 "1 V" 1 "1 kHz" 0 "0" 0 "0" 0 "0" 0 "0" 0>
  <R R1 1 250 150 -26 15 0 0 "1k" 1 "26.85" 0 "0.0" 0 "0.0" 0 "26.85" 0 "US" 0>
  <C C1 1 350 250 17 -26 0 1 "159n" 1 "" 0 "neutral" 0>
  <GND * 1 150 330 0 0 0 0>
  <GND * 1 350 330 0 0 0 0>
  <.AC AC1 1 150 420 0 41 0 0 "lin" 1 "10 Hz" 1 "100 kHz" 1 "50" 1 "no" 0>
</Components>
<Wires>
  <150 150 220 150 "in" 150 120 40 "">
  <280 150 350 150 "out" 330 110 40 "">
  <150 150 150 220 "" 0 0 0 "">
  <350 150 350 220 "" 0 0 0 "">
  <150 280 150 330 "" 0 0 0 "">
  <350 280 350 330 "" 0 0 0 "">
</Wires>
<Diagrams>
</Diagrams>
<Paintings>
</Paintings>
