cmake_minimum_required(VERSION 3.5)
project(c_plugin_example C)

add_library(c_plugin_example SHARED plugin.c)
set_target_properties(c_plugin_example PROPERTIES
  OUTPUT_NAME "c_plugin_example"
  PREFIX ""
  # Place produced artifacts in the source plugin directory so the host can
  # discover them directly without CI copy/rename steps.
  LIBRARY_OUTPUT_DIRECTORY "${CMAKE_SOURCE_DIR}"
  RUNTIME_OUTPUT_DIRECTORY "${CMAKE_SOURCE_DIR}"
  ARCHIVE_OUTPUT_DIRECTORY "${CMAKE_SOURCE_DIR}"
)

# On some platforms you may want to set position independent code and versioning.
if(NOT WIN32)
  set_target_properties(c_plugin_example PROPERTIES POSITION_INDEPENDENT_CODE ON)
endif()
