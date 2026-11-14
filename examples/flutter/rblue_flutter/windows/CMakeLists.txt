# The Flutter tooling requires that developers have a version of Visual Studio
# installed that includes CMake 3.14 or later. Keep this aligned with Flutter's
# Windows plugin_ffi template so consumers are not forced onto a newer CMake.
cmake_minimum_required(VERSION 3.14)

set(PROJECT_NAME "rblue_flutter")
project(${PROJECT_NAME} LANGUAGES NONE)

# rblue_c.dll is built separately from the Rust workspace for the x64 MSVC
# target and packaged here. This FFI-only plugin has no Flutter plugin class.
# RBLUE_FLUTTER_CMAKE_VALIDATE is used only by host-independent CMake syntax
# checks; production Flutter builds must satisfy the Windows/MSVC guards.
if(NOT WIN32 AND NOT RBLUE_FLUTTER_CMAKE_VALIDATE)
  message(FATAL_ERROR
    "rblue_flutter's Windows plugin must be configured by a Windows Flutter "
    "build. Configure the example on Windows with Flutter 3.41.7 or later.")
endif()

if(NOT MSVC AND NOT RBLUE_FLUTTER_CMAKE_VALIDATE)
  message(FATAL_ERROR
    "rblue_flutter currently supports only the MSVC Windows toolchain "
    "(x86_64-pc-windows-msvc).")
endif()

if(NOT RBLUE_FLUTTER_CMAKE_VALIDATE)
  if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
    message(FATAL_ERROR
      "rblue_flutter currently supports only 64-bit Windows (x64). "
      "Select an x64 Visual Studio generator/toolchain.")
  endif()
  if(CMAKE_GENERATOR_PLATFORM AND NOT CMAKE_GENERATOR_PLATFORM STREQUAL "x64")
    message(FATAL_ERROR
      "rblue_flutter packages an x64 DLL, but CMAKE_GENERATOR_PLATFORM is "
      "${CMAKE_GENERATOR_PLATFORM}. Select the x64 platform.")
  endif()
  if(DEFINED ENV{Platform} AND NOT "$ENV{Platform}" STREQUAL "x64" AND
      NOT "$ENV{Platform}" STREQUAL "X64")
    message(FATAL_ERROR
      "rblue_flutter packages an x64 DLL, but the Visual Studio Platform is "
      "$ENV{Platform}. Select x64.")
  endif()
endif()

set(RBLUE_C_DLL "${CMAKE_CURRENT_SOURCE_DIR}/x64/rblue_c.dll"
  CACHE FILEPATH "Absolute path to the prebuilt x64 MSVC rblue_c.dll")
get_filename_component(RBLUE_C_DLL "${RBLUE_C_DLL}" ABSOLUTE
  BASE_DIR "${CMAKE_CURRENT_SOURCE_DIR}")

if(NOT EXISTS "${RBLUE_C_DLL}")
  message(FATAL_ERROR
    "Required rblue Windows FFI artifact was not found:\n"
    "  ${RBLUE_C_DLL}\n"
    "Build crates/rblue-c on a Windows x64 MSVC host with:\n"
    "  cargo build --locked --release --target x86_64-pc-windows-msvc "
    "--no-default-features --features winrt -p rblue-c\n"
    "Then copy target/x86_64-pc-windows-msvc/release/rblue_c.dll to:\n"
    "  ${CMAKE_CURRENT_SOURCE_DIR}/x64/rblue_c.dll\n"
    "For a local build only, RBLUE_C_DLL may point at an equivalent DLL.")
endif()

if(IS_DIRECTORY "${RBLUE_C_DLL}")
  message(FATAL_ERROR "RBLUE_C_DLL must name a file, not a directory: ${RBLUE_C_DLL}")
endif()

get_filename_component(RBLUE_C_DLL_NAME "${RBLUE_C_DLL}" NAME)
string(TOLOWER "${RBLUE_C_DLL_NAME}" RBLUE_C_DLL_NAME_LOWER)
if(NOT RBLUE_C_DLL_NAME_LOWER STREQUAL "rblue_c.dll")
  message(FATAL_ERROR
    "RBLUE_C_DLL must be named rblue_c.dll because Dart opens that exact "
    "runtime name; got: ${RBLUE_C_DLL_NAME}")
endif()

message(STATUS "rblue_flutter: bundling x64 MSVC FFI DLL: ${RBLUE_C_DLL}")

# Flutter's generated_plugins.cmake appends this package-named variable to
# PLUGIN_BUNDLED_LIBRARIES. The app template installs every entry beside the
# runner executable, where DynamicLibrary.open('rblue_c.dll') can resolve it.
if(RBLUE_FLUTTER_CMAKE_VALIDATE)
  set(rblue_flutter_bundled_libraries "${RBLUE_C_DLL}")
else()
  set(rblue_flutter_bundled_libraries
    "${RBLUE_C_DLL}"
    PARENT_SCOPE
  )
endif()
