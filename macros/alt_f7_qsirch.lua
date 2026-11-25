-- FarQsirch Alt+F7 Macro
-- Copy this file to: %FARPROFILE%\Macros\scripts\alt_f7_qsirch.lua
--
-- Alt+F7       -> Opens FarQsirch Unified Search (Local + QNAP Qsirch 7)
-- Shift+Alt+F7 -> Opens standard built-in FAR Find File dialog

local FAR_QSIRCH_GUID = "6F4A9C12-8E31-4D7B-9A24-5C1E7B3D8F01"
local FAR_QSIRCH_MENU = "6F4A9C12-8E31-4D7B-9A24-5C1E7B3D8F02"

Macro {
  area = "Shell";
  key = "AltF7";
  description = "Unified Search (Local + QNAP Qsirch 7)";
  action = function()
    if Plugin.Exist(FAR_QSIRCH_GUID) then
      Plugin.Menu(FAR_QSIRCH_GUID, FAR_QSIRCH_MENU)
    else
      Keys("AltF7")
    end
  end;
}

Macro {
  area = "Shell";
  key = "ShiftAltF7";
  description = "Standard FAR Find File (fallback)";
  action = function()
    Keys("AltF7")
  end;
}
