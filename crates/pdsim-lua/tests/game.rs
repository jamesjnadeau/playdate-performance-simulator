//! End-to-end tests: load small games and run frames.

use pdsim_core::{Calibration, Category, FrameStats};
use pdsim_device::{Device, Game};
use pdsim_lua::{LuaGame, LuaOptions};
use std::path::{Path, PathBuf};

struct TempGame {
    dir: PathBuf,
}

impl TempGame {
    fn new(name: &str, main: &str) -> TempGame {
        let dir = std::env::temp_dir().join(format!("pdsim-lua-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("game/main.lua"), main).unwrap();
        TempGame { dir }
    }

    fn file(&self, rel: &str, contents: &str) -> &Self {
        let p = self.dir.join("game").join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
        self
    }

    fn load_with(&self, sdk: Option<PathBuf>) -> Result<LuaGame, String> {
        let dev = Device::new(
            Calibration::default_estimates(),
            &self.dir.join("game"),
            &self.dir.join("data"),
            sdk,
        )
        .shared();
        LuaGame::load(dev, LuaOptions::default()).map_err(|e| e.0)
    }

    fn load(&self) -> LuaGame {
        self.load_with(None).unwrap()
    }
}

impl Drop for TempGame {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run(g: &mut LuaGame, n: usize) -> Vec<FrameStats> {
    (0..n).map(|_| g.frame(None).unwrap()).collect()
}

fn sdk() -> Option<PathBuf> {
    Device::sdk_from_env()
}

#[test]
fn hello_world_runs_and_charges_lua() {
    let t = TempGame::new(
        "hello",
        r#"
        local n = 0
        function playdate.update()
            n += 1
            local s = 0
            for i = 1, 1000 do s = s + i end
            playdate.graphics.drawText("hi " .. n, 10, 10)
        end
        "#,
    );
    let mut g = t.load();
    let frames = run(&mut g, 3);
    for f in &frames {
        assert!(
            f.cycles.get(Category::Lua) > 10_000,
            "lua cycles {}",
            f.cycles.get(Category::Lua)
        );
        assert!(f.cycles.get(Category::Gfx) > 0);
        assert!(f.rows_updated > 0);
        assert!(!f.over_budget);
    }
    // The frame-count text changed every frame, so the display was updated.
    assert!(frames[2].cycles.get(Category::Display) > 0);
}

#[test]
fn doubling_work_doubles_lua_time() {
    let src = |n: u32| {
        format!(
            "function work() local s = 0 for i = 1, {n} do s = s + math.sqrt(i) end return s end
             function playdate.update() work() end"
        )
    };
    let a = TempGame::new("double-a", &src(20_000));
    let b = TempGame::new("double-b", &src(40_000));
    let fa = run(&mut a.load(), 2).pop().unwrap();
    let fb = run(&mut b.load(), 2).pop().unwrap();
    let ratio = fb.cycles.get(Category::Lua) as f64 / fa.cycles.get(Category::Lua) as f64;
    assert!((ratio - 2.0).abs() < 0.05, "ratio {ratio}");
}

#[test]
fn heavy_frames_are_flagged_over_budget() {
    let t = TempGame::new(
        "spike",
        r#"
        local f = 0
        function playdate.update()
            f += 1
            if f == 3 then
                local t = {}
                for i = 1, 300000 do t[i] = i * 2 end
            end
        end
        "#,
    );
    let mut g = t.load();
    let frames = run(&mut g, 5);
    assert!(!frames[1].over_budget);
    assert!(frames[2].over_budget, "work {} ms", frames[2].work_ms);
    assert!(frames[2].dropped >= 1);
    assert!(frames[2].cycles.get(Category::Gc) > 0);
    assert!(!frames[3].over_budget);
}

#[test]
fn hotspots_name_the_hot_function() {
    let t = TempGame::new(
        "hot",
        r#"
        local function cold() local s = 0 for i = 1, 100 do s = s + i end return s end
        local function hot() local s = 0 for i = 1, 50000 do s = s + i end return s end
        function playdate.update() cold() hot() end
        "#,
    );
    let mut g = t.load();
    run(&mut g, 3);
    let top = g.device().borrow().hotspots.top(5);
    assert!(top[0].name.starts_with("hot main.lua:3"), "{:?}", top);
}

#[test]
fn time_follows_the_device_clock() {
    let t = TempGame::new(
        "time",
        r#"
        times = {}
        function playdate.update()
            playdate.resetElapsedTime()
            local s = 0
            for i = 1, 20000 do s = s + i end
            times[#times + 1] = playdate.getElapsedTime()
            ms = playdate.getCurrentTimeMilliseconds()
        end
        "#,
    );
    let mut g = t.load();
    let frames = run(&mut g, 2);
    let lua = g.lua();
    let times: Vec<f64> = lua
        .globals()
        .get::<mlua::Table>("times")
        .unwrap()
        .sequence_values()
        .map(|v| v.unwrap())
        .collect();
    // Elapsed time is device time: positive, and about the frame's Lua time.
    assert!(times[0] > 0.0);
    let lua_ms = frames[1].categories[&Category::Lua];
    assert!(
        times[1] * 1000.0 <= lua_ms + 0.01 && times[1] * 1000.0 > lua_ms * 0.8,
        "{} vs {}",
        times[1] * 1000.0,
        lua_ms
    );
    let ms: i64 = lua.globals().get("ms").unwrap();
    assert!(ms >= 33, "second frame starts after the first refresh: {ms}");
}

#[test]
fn import_compound_ops_and_files() {
    let t = TempGame::new(
        "import",
        r#"
        import "lib/util"
        import "lib/util"
        local x = 1
        x += util.inc
        x *= 10
        result = x
        playdate.datastore.write({score = x}, "save")
        loaded = playdate.datastore.read("save")
        function playdate.update() end
        "#,
    );
    t.file("lib/util.lua", "util = {inc = 2}\ncount = (count or 0) + 1\n");
    let g = t.load();
    let lua = g.lua();
    assert_eq!(lua.globals().get::<i64>("result").unwrap(), 30);
    assert_eq!(lua.globals().get::<i64>("count").unwrap(), 1, "imported once");
    let loaded: mlua::Table = lua.globals().get("loaded").unwrap();
    assert_eq!(loaded.get::<i64>("score").unwrap(), 30);
    assert!(t.dir.join("data/save.json").exists());
}

#[test]
fn sprites_draw_and_collide() {
    let t = TempGame::new(
        "sprites",
        r#"
        import "CoreLibs/object"
        import "CoreLibs/sprites"
        local gfx = playdate.graphics
        local img = gfx.image.new(10, 10, gfx.kColorBlack)
        local a = gfx.sprite.new(img)
        a:moveTo(50, 50)
        a:setCollideRect(0, 0, 10, 10)
        a:add()
        local wall = gfx.sprite.new()
        wall:setSize(10, 100)
        wall:moveTo(100, 50)
        wall:setCollideRect(0, 0, 10, 100)
        wall:add()
        updates = 0
        function a:update() updates += 1 end
        function playdate.update()
            gfx.sprite.update()
            ax, ay, cols, n = a:moveWithCollisions(200, 50)
        end
        "#,
    );
    let mut g = t.load();
    run(&mut g, 1);
    let lua = g.lua();
    assert_eq!(lua.globals().get::<i64>("updates").unwrap(), 1);
    assert_eq!(lua.globals().get::<i64>("n").unwrap(), 1);
    let ax: f64 = lua.globals().get("ax").unwrap();
    assert!((ax - 90.0).abs() < 1e-3, "stopped at the wall: {ax}");
    let dev = g.device().borrow();
    assert!(!dev.gfx.frame().get(50, 50), "sprite drawn black");
    assert!(dev.gfx.frame().get(150, 150), "background still white");
}

#[test]
fn errors_stop_the_game_with_a_message() {
    let t = TempGame::new("err", "function playdate.update() local x = nil; x.y = 1 end");
    let mut g = t.load();
    let e = g.frame(None).unwrap_err();
    assert!(e.0.contains("main.lua:1"), "{}", e.0);
    assert!(g.frame(None).is_err());
}

#[test]
fn input_callbacks_and_queries() {
    use pdsim_device::buttons;
    let t = TempGame::new(
        "input",
        r#"
        downs, cranks = 0, 0
        function playdate.AButtonDown() downs += 1 end
        function playdate.cranked(c) cranks += c end
        function playdate.update()
            pressed = playdate.buttonIsPressed(playdate.kButtonA)
            just = playdate.buttonJustPressed("a")
        end
        "#,
    );
    let mut g = t.load();
    g.device().borrow_mut().input.press(buttons::A);
    g.device().borrow_mut().input.crank_by(15.0);
    g.frame(None).unwrap();
    let lua = g.lua();
    assert_eq!(lua.globals().get::<i64>("downs").unwrap(), 1);
    assert_eq!(lua.globals().get::<f64>("cranks").unwrap(), 15.0);
    assert!(lua.globals().get::<bool>("just").unwrap());
    g.frame(None).unwrap();
    assert!(g.lua().globals().get::<bool>("pressed").unwrap());
    assert!(!g.lua().globals().get::<bool>("just").unwrap());
}

#[test]
fn example_game_runs() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lua-stress/Source");
    let data = std::env::temp_dir().join(format!("pdsim-example-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    let dev = Device::new(Calibration::default_estimates(), &src, &data, sdk()).shared();
    let mut g = LuaGame::load(dev, LuaOptions::default()).unwrap();
    let frames: Vec<FrameStats> = (0..61).map(|_| g.frame(None).unwrap()).collect();
    // The game counts frames from 1, so its 60th frame is index 59.
    assert!(frames[59].over_budget, "spike frame: {} ms", frames[59].work_ms);
    assert!(frames[59].cycles.get(Category::Gc) > 0);
    assert!(!frames[30].over_budget, "normal frame: {} ms", frames[30].work_ms);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn sdk_corelibs_load_when_available() {
    let Some(sdk) = sdk() else {
        eprintln!("PLAYDATE_SDK_PATH not set; skipping");
        return;
    };
    let t = TempGame::new(
        "corelibs",
        r#"
        import "CoreLibs/object"
        import "CoreLibs/graphics"
        import "CoreLibs/sprites"
        import "CoreLibs/timer"
        import "CoreLibs/frameTimer"
        import "CoreLibs/crank"
        import "CoreLibs/animator"
        import "CoreLibs/easing"
        import "CoreLibs/ui"
        import "CoreLibs/nineslice"
        local gfx = playdate.graphics
        class("Thing").extends(gfx.sprite)
        function Thing:init() Thing.super.init(self) self:setSize(10, 10) self:add() end
        local thing = Thing()
        fired = false
        playdate.timer.performAfterDelay(50, function() fired = true end)
        function playdate.update()
            gfx.sprite.update()
            gfx.drawTextInRect("Hello from CoreLibs", 10, 10, 200, 40)
            gfx.drawCircleAtPoint(100, 100, 20)
            playdate.timer.updateTimers()
        end
        "#,
    );
    let mut g = t.load_with(Some(sdk)).unwrap();
    run(&mut g, 4);
    assert!(g.lua().globals().get::<bool>("fired").unwrap());
}

/// Checks the simulator defines the functions the SDK's `__stub.lua` lists
/// (the full native Lua API). Prints what is missing.
#[test]
fn api_coverage_against_sdk_stub() {
    let Some(sdk) = sdk() else {
        eprintln!("PLAYDATE_SDK_PATH not set; skipping");
        return;
    };
    let stub = std::fs::read_to_string(sdk.join("CoreLibs/__stub.lua")).unwrap();
    let names: Vec<String> = stub
        .lines()
        .filter_map(|l| l.strip_prefix("function "))
        .filter_map(|l| l.split('(').next())
        .map(|s| s.trim().to_string())
        .collect();
    let t = TempGame::new(
        "coverage",
        r#"
        import "CoreLibs/object"
        import "CoreLibs/graphics"
        import "CoreLibs/sprites"
        import "CoreLibs/timer"
        import "CoreLibs/frameTimer"
        import "CoreLibs/crank"
        import "CoreLibs/animation"
        import "CoreLibs/animator"
        import "CoreLibs/easing"
        import "CoreLibs/math"
        import "CoreLibs/string"
        import "CoreLibs/nineslice"
        import "CoreLibs/ui"
        import "CoreLibs/keyboard"
        import "CoreLibs/qrcode"
        function playdate.update() end
        function has(path)
            local cur = _G
            for part in path:gmatch("[^%.:]+") do
                if type(cur) ~= "table" then return false end
                local ok, v = pcall(function() return cur[part] end)
                if not ok or v == nil then return false end
                cur = v
            end
            return true
        end
        "#,
    );
    let g = t.load_with(Some(sdk)).unwrap();
    let has: mlua::Function = g.lua().globals().get("has").unwrap();
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| !has.call::<bool>(n.as_str()).unwrap())
        .collect();
    let pct = 100.0 * (names.len() - missing.len()) as f64 / names.len() as f64;
    eprintln!(
        "API coverage: {:.1}% of {} stub functions; missing:",
        pct,
        names.len()
    );
    for m in &missing {
        eprintln!("  {m}");
    }
    assert!(pct >= 90.0, "coverage {pct:.1}%");
}

#[test]
fn runs_are_deterministic() {
    // Same game, same input: identical cycles every frame (needed for CI
    // regression checks). Uses table iteration order and math.random.
    let src = r#"
        local t = {}
        for i = 1, 200 do t["key" .. i] = i end
        function playdate.update()
            local s = 0
            for k, v in pairs(t) do
                if v % 7 == 0 then break end
                s = s + #k
            end
            for i = 1, math.random(100, 2000) do s = s + i end
        end
    "#;
    let a = TempGame::new("det-a", src);
    let b = TempGame::new("det-b", src);
    let (mut ga, mut gb) = (a.load(), b.load());
    for _ in 0..5 {
        assert_eq!(ga.frame(None).unwrap().cycles, gb.frame(None).unwrap().cycles);
    }
}
