cmake_minimum_required(VERSION 3.20)
project(recast-exporter LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT LLVM_DIR)
  find_program(LLVM_CONFIG_EXECUTABLE NAMES llvm-config llvm-config-21 llvm-config-20 llvm-config-19 llvm-config-18)
  if(NOT LLVM_CONFIG_EXECUTABLE)
    message(FATAL_ERROR "llvm-config was not found; install LLVM and Clang development headers, or pass -DLLVM_DIR")
  endif()

  execute_process(
    COMMAND ${LLVM_CONFIG_EXECUTABLE} --cmakedir
    OUTPUT_VARIABLE LLVM_CMAKE_DIR
    OUTPUT_STRIP_TRAILING_WHITESPACE
  )
  get_filename_component(LLVM_PREFIX "${LLVM_CMAKE_DIR}/../../.." ABSOLUTE)
  list(APPEND CMAKE_PREFIX_PATH "${LLVM_PREFIX}")
endif()

find_package(LLVM REQUIRED CONFIG)
find_package(Clang REQUIRED CONFIG)

message(STATUS "recast exporter uses LLVM ${LLVM_PACKAGE_VERSION}")

add_executable(recast-exporter src/main.cpp)
target_include_directories(recast-exporter PRIVATE ${LLVM_INCLUDE_DIRS} ${CLANG_INCLUDE_DIRS})
target_link_libraries(recast-exporter PRIVATE clang-cpp LLVM)
target_compile_definitions(
  recast-exporter
  PRIVATE __STDC_CONSTANT_MACROS __STDC_FORMAT_MACROS __STDC_LIMIT_MACROS
)
