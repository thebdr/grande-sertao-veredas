// Oficina's C++ side, inside the bridge seam: what Qt wants done in C++ before and around the QML
// engine (the docking library's front end, and in the tests' build the test server).
#pragma once

#include <cstdint>

#include "rust/cxx.h"

// Run the application until its window closes; `args` are the process's arguments.
::std::int32_t oficina_run(::rust::Vec<::rust::String> args);
