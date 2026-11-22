// The shell's C++ side, inside the bridge seam: what Qt wants done in C++ before and around the
// QML engine (WebEngine's initialisation, the docking library's front end, the test server).
#pragma once

#include <cstdint>

#include "rust/cxx.h"

// Run the application until its window closes; `args` are the process's arguments.
::std::int32_t oficina_run(::rust::Vec<::rust::String> args);
