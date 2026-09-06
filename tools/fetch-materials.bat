@echo off
rem Fetches the optional scanned/recorded assets: PBR materials and sounds.
rem
rem This is the Windows twin of tools/fetch-materials.sh. KEEP IN SYNC: any
rem material or sound added to one must be added to the other. The lists
rem below mirror the .sh exactly; only the plumbing differs (curl.exe and
rem tar.exe ship with Windows 10+, so nothing needs installing).
rem
rem Everything fetched is CC0 1.0 — public domain, no attribution required.
rem (The project is FOSS and CC-BY with credit would also do, but everything
rem worth having has turned up CC0 so far.) Materials come from ambientCG,
rem sounds from OpenGameArt and Freesound. The freesound.org entries point at
rem cdn previews (128kbps mp3) because the originals sit behind a login; the
rem licence is the sound's licence either way. Both directories are gitignored
rem and both are optional: the game generates procedural stand-ins for
rem anything missing.
rem
rem   tools\fetch-materials.bat          fetch anything missing
rem   tools\fetch-materials.bat --force  re-fetch everything

setlocal enabledelayedexpansion
cd /d "%~dp0.."

set "DEST=assets\materials"
set "RESOLUTION=2K-JPG"
set "FORCE=%1"

rem The set the renderer looks for — mirror of MATERIALS in the .sh.
set MATERIALS=Asphalt031 PavingStones138 Concrete034 Concrete046 Bricks097 Bricks104 Bricks075A PaintedPlaster006 Gravel023 Grass005

if not exist "%DEST%" mkdir "%DEST%"
for %%M in (%MATERIALS%) do (
    if exist "%DEST%\%%M" if not "%FORCE%"=="--force" (
        echo have    %%M
    ) else (
        call :fetch_material %%M
    )
    if not exist "%DEST%\%%M" call :fetch_material %%M
)

rem ---------------------------------------------------------------- sounds ----
rem One entry per sound bank name — mirror of SOUNDS in the .sh. The file
rem keeps its source extension; audio::files tries wav/flac/ogg/mp3 in turn.
set "SOUNDS_DEST=assets\sounds"
if not exist "%SOUNDS_DEST%" mkdir "%SOUNDS_DEST%"

call :fetch_sound boing flac "https://opengameart.org/sites/default/files/boing.flac"
call :fetch_sound crash ogg "https://opengameart.org/sites/default/files/qubodup-crash.ogg"
call :fetch_sound honk wav "https://opengameart.org/sites/default/files/bicycle-horn-1.wav"
call :fetch_sound explosion mp3 "https://opengameart.org/sites/default/files/Chunky%%20Explosion.mp3"
call :fetch_sound birdsong wav "https://opengameart.org/sites/default/files/park_ambience_birds.wav"
call :fetch_sound spray wav "https://opengameart.org/sites/default/files/park_ambience_river.wav"
call :fetch_sound uproar ogg "https://opengameart.org/sites/default/files/crowd_shouting_0.ogg"
rem The cheer, whistled by actual people, three takes — mirror of the .sh.
call :fetch_sound whistle-0 mp3 "https://cdn.freesound.org/previews/146/146887_197046-hq.mp3"
call :fetch_sound whistle-1 mp3 "https://cdn.freesound.org/previews/411/411578_7994683-hq.mp3"
call :fetch_sound whistle-2 mp3 "https://cdn.freesound.org/previews/411/411062_7963328-hq.mp3"
call :fetch_sound wheee mp3 "https://cdn.freesound.org/previews/497/497092_942821-hq.mp3"
call :fetch_sound sproing mp3 "https://cdn.freesound.org/previews/540/540790_11537497-hq.mp3"

rem The car pack zip — mirror of CAR_PACK in the .sh.
set "NEED_PACK="
if not exist "%SOUNDS_DEST%\engine.ogg" set NEED_PACK=1
if not exist "%SOUNDS_DEST%\car-door.ogg" set NEED_PACK=1
if "%FORCE%"=="--force" set NEED_PACK=1
if defined NEED_PACK (
    echo fetch   car sound pack
    curl -fsSL --retry 3 --retry-delay 2 -o "%TEMP%\carpack.zip" "https://opengameart.org/sites/default/files/car_sound_effects_pack.zip"
    if not errorlevel 1 (
        tar -xf "%TEMP%\carpack.zip" -C "%TEMP%" Car_Engine_Loop.ogg Car_Door_Close.ogg
        move /y "%TEMP%\Car_Engine_Loop.ogg" "%SOUNDS_DEST%\engine.ogg" >nul
        move /y "%TEMP%\Car_Door_Close.ogg" "%SOUNDS_DEST%\car-door.ogg" >nul
    ) else (
        echo         failed; skipping - the game synthesises them instead 1>&2
    )
    del /q "%TEMP%\carpack.zip" 2>nul
)

rem The creature pack zip — mirror of CREATURE_PACK in the .sh: the taunt
rem rotation's recordable half. Raspberry, fart and sorry have no good CC0
rem recording yet and stay synthesised.
set "NEED_CREATURES="
if not exist "%SOUNDS_DEST%\cough.ogg" set NEED_CREATURES=1
if not exist "%SOUNDS_DEST%\spit.ogg" set NEED_CREATURES=1
if "%FORCE%"=="--force" set NEED_CREATURES=1
if defined NEED_CREATURES (
    echo fetch   creature sound pack
    curl -fsSL --retry 3 --retry-delay 2 -o "%TEMP%\creaturepack.zip" "https://opengameart.org/sites/default/files/80-CC0-creature-SFX_0.zip"
    if not errorlevel 1 (
        tar -xf "%TEMP%\creaturepack.zip" -C "%TEMP%" cough_03.ogg spit_01.ogg
        move /y "%TEMP%\cough_03.ogg" "%SOUNDS_DEST%\cough.ogg" >nul
        move /y "%TEMP%\spit_01.ogg" "%SOUNDS_DEST%\spit.ogg" >nul
    ) else (
        echo         failed; skipping - the game synthesises them instead 1>&2
    )
    del /q "%TEMP%\creaturepack.zip" 2>nul
)

rem The SFX hundred pack — mirror of SFX100_PACK in the .sh: footsteps and
rem the traffic bed (the highway loop is the city ambience).
set "NEED_SFX100="
if not exist "%SOUNDS_DEST%\footstep.ogg" set NEED_SFX100=1
if not exist "%SOUNDS_DEST%\ambience.ogg" set NEED_SFX100=1
if "%FORCE%"=="--force" set NEED_SFX100=1
if defined NEED_SFX100 (
    echo fetch   sfx hundred pack
    curl -fsSL --retry 3 --retry-delay 2 -o "%TEMP%\sfx100pack.zip" "https://opengameart.org/sites/default/files/sfx_100_v2.zip"
    if not errorlevel 1 (
        tar -xf "%TEMP%\sfx100pack.zip" -C "%TEMP%" sfx100v2_footstep_01.ogg sfx100v2_loop_highway.ogg
        move /y "%TEMP%\sfx100v2_footstep_01.ogg" "%SOUNDS_DEST%\footstep.ogg" >nul
        move /y "%TEMP%\sfx100v2_loop_highway.ogg" "%SOUNDS_DEST%\ambience.ogg" >nul
    ) else (
        echo         failed; skipping - the game synthesises them instead 1>&2
    )
    del /q "%TEMP%\sfx100pack.zip" 2>nul
)

echo done
exit /b 0

:fetch_material
echo fetch   %1
curl -fsSL --retry 3 --retry-delay 2 -o "%TEMP%\%1.zip" "https://ambientcg.com/get?file=%1_%RESOLUTION%.zip"
if errorlevel 1 (
    echo         failed; skipping 1>&2
    del /q "%TEMP%\%1.zip" 2>nul
    exit /b 0
)
if exist "%DEST%\%1" rmdir /s /q "%DEST%\%1"
mkdir "%DEST%\%1"
tar -xf "%TEMP%\%1.zip" -C "%DEST%\%1"
del /q "%TEMP%\%1.zip" 2>nul
rem Drop what the renderer will never read — mirror of the .sh's find/-delete.
for /r "%DEST%\%1" %%F in (*) do if /i not "%%~xF"==".jpg" del /q "%%F"
exit /b 0

:fetch_sound
if exist "%SOUNDS_DEST%\%1.%2" if not "%FORCE%"=="--force" (
    echo have    %1
    exit /b 0
)
echo fetch   %1
curl -fsSL --retry 3 --retry-delay 2 -o "%SOUNDS_DEST%\%1.%2" %3
if errorlevel 1 (
    echo         failed; skipping - the game synthesises it instead 1>&2
    del /q "%SOUNDS_DEST%\%1.%2" 2>nul
)
exit /b 0
