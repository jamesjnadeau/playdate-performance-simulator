Minimal stand-ins for the SDK's CoreLibs, written for the simulator. They are
used only when no Playdate SDK is available (`PLAYDATE_SDK_PATH` unset); with
an SDK, `import "CoreLibs/..."` loads Panic's real CoreLibs from the SDK, which
gives more accurate costs. These files are original code, not copies of the
SDK's.
