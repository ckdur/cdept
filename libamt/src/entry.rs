/*Entry definition file
An "Entry" is any object inside of an element.
The entry can be any folder having a file or group of files with a function.
Such function can be:
- Layout
- Schematic
- Netlist
- Simulation
*/

enum entry_type {
    // Undefined
    Undefined,
    // Layout types
    Layout_Mag,
    Layout_GDS,
    // Schematics
    Schematic_Qucs,
    Schematic_Xschem,
    Schematic_Xcircuit,
    // Netlist
    Netlist_Spice,
    Netlist_CDL,
    // Simulation
    Simulation_Default
}

#[derive(Default)]
struct entry {
    typ: entry_type = entry_type.Undefined,
}
