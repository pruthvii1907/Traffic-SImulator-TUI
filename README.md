**traffic-tui**  
A lively, procedurally generated **ASCII traffic simulator** for your terminal, written in Rust with [ratatui.](https://ratatui.rs "https://ratatui.rs")  
Drive nothing, watch everything: a scrollable dark-and-white city full of dark green cars, pedestrians, traffic lights, red-light runners, crashes, tow trucks, and a city that keeps throwing random events at you.  
```  
== oo ==<       <       == oo ==<<]     <       <       < == oo ==  
 ==    ==>       >       ==    ==> [> [> ==    ==>       >      [> ==  
   ||||                    ||||            ||||                  ||||  
   v  ^  [][][][]  ##%%##  v  ^  ()()()()  v  ^              ()()()  
         [][][][]  %%##%%        ()()()()                    ()()()  
   
```  
**Features**  
**Procedural city**  
- Every city is generated from a random seed: uneven road spacing, blocks from 6 to \~19 tiles wide, and big blocks recursively split into lots with alleys.  
- Lots become buildings, bright white towers, parks, or open plazas.  
- The default map is 14 x 10 roads (about 200 x 130 tiles), far bigger than a terminal, so you scroll around it.  
- Minimal palette: near-black roads, grey sidewalks and buildings, white pedestrians, dark green cars.  
**Traffic simulation**  
- Cars follow the Intelligent Driver Model (IDM) with per-driver speed, aggressiveness and recklessness.  
- Signalised intersections with a staggered "green wave", yellow and red phases.  
- Intersection reservation so cars don't drive through each other (unless somebody runs a red).  
- Left, right and straight turns, zebra crossings, pedestrians who wait at the kerb (or don't).  
- Gridlock breaker: a tow truck clears cars stuck for a minute.  
**Traffic log**  
   
 A live log of everything that goes wrong, with street names like `L Ave & 3rd St`:  
- red-light runs (how long it had been red, and whether the driver didn't look)  
- rear-end crashes (including distracted drivers) and junction collisions  
- oil-slick spin-outs  
- jaywalking pedestrians  
- breakdowns, tow trucks, gridlock starting and clearing  
**A city with a mind of its own**  
   
 Random events fire on their own every so often:  
  
|  |  |  
| - | - |  
| **Event** |  **What happens** |  
| Rush hour |  1.7x the cars flood the roads |  
| Rain |  Slower, twitchier drivers, rain streaks on screen |  
| Crowd surge |  A big crowd of pedestrians spills onto the streets |  
| Police chase |  One car flees at high speed and runs reds |  
| Power outage |  A cluster of signals goes dark and flashes yellow |  
| Breakdown |  A car stalls with hazard lights |  
| Oil spill |  A slick that makes cars skid out |  
  
   
**Getting started**  
You need a recent stable Rust toolchain and a terminal with 24-bit colour support.  
```  
git clone <your-repo-url>  
 cd traffic-tui  
 cargo run --release  
   
```  
A terminal of roughly 120 x 40 or larger works best. The map window follows your terminal size.  
```  
cargo run --release -- --snapshot 30  
 cargo run --release -- --snapshot 60 --cars 120 --peds 40 --chaos 45  
 cargo run --release -- --snapshot 60 --quiet 1     # stats and log only, no map  
   
```  
**Controls**  
  
|  |  |  
| - | - |  
| **Key** |  **Action** |  
| `Up` / `Down` or `j` / `k` |  Pick a slider (or press `1`-`7`) |  
| `Left` / `Right` or `h` / `l` |  Turn the selected knob |  
| `Shift` + arrows or `H` / `L` |  Big steps |  
| `w` `a` `s` `d` |  Scroll the map (`W` `A` `S` `D` scroll 4x faster) |  
| `c` |  Centre the camera |  
| `g` |  Jump to the latest incident |  
| `e` |  Trigger a random event now |  
| `Space` or `p` |  Pause |  
| `r` |  Generate a new city |  
| `q` / `Esc` |  Quit |  
  
   
**Sliders**  
  
|  |  |  |  
| - | - | - |  
| **Slider** |  **Range** |  **What it does** |  
| Cars |  0-600 |  Target number of cars |  
| Chaos |  0-100% |  Aggressive driving, red-light runners, distraction, jaywalkers |  
| Pedestrians |  0-300 |  Target number of pedestrians |  
| Speed limit |  20-120 km/h |  Base speed of traffic |  
| Green time |  3-30 s |  Length of each green phase |  
| Sim speed |  0.25-4x |  Simulation speed |  
| Events |  0-100% |  How often random events happen (0 turns them off) |  
  
   
Set **Chaos** and **Events** to 0 for a calm, orderly city with no incidents.  
**Project layout**  
```  
src/  
   sim.rs      core simulation: city generation, cars, pedestrians, lights, incidents, events  
   render.rs   turns the world into a grid of ASCII cells (UI-agnostic), draws the visible window  
   app.rs      shared app state: sliders, camera, history graphs  
   ui.rs       ratatui drawing: map, traffic log, control panel  
   main.rs     terminal front end, key handling, headless --snapshot mode  
   wasm.rs     minimal WebAssembly bindings (plain extern "C", no wasm-bindgen)  
   lib.rs      library root (crate name: traffic)  
 tests/        generation stress tests and incident-rate checks  
   
```  
The simulation and renderer know nothing about the terminal, so any front end can drive them. The only dependency is `ratatui`; the RNG is a tiny built-in xorshift generator, which keeps the core wasm-friendly.  
**WebAssembly**  
`wasm.rs` exposes a small C ABI (`init`, `update`, `render`, `set_view`, `scroll`, `set_param`, `log_ptr`, `stat`, ...). A JS front end calls `update(dt)` and `render()` each animation frame and reads the frame straight out of wasm memory. Each cell is 8 bytes: `[ascii, fg_r, fg_g, fg_b, bg_r, bg_g, bg_b, bold]`.  
**Tests**  
```  
cargo test --release  
   
```  
The tests simulate many seeds and full-size cities, check that scrolling is clamped, and measure incident rates at several chaos and event levels.  
**Ideas for later**  
- An endless map generated in chunks, simulating only the cars near the camera  
- T-junctions, dead ends and curved roads  
- Buses, emergency vehicles and traffic that routes to destinations  
- A save/replay system for interesting crashes  
**License**  
Released under the [MIT License.](LICENSE "LICENSE")  
