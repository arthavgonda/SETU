cmake_minimum_required(VERSION 3.24)
project(Setu VERSION 1.8.3 LANGUAGES CXX)

set(SETU_PRODUCT_NAME "Setu" CACHE STRING "Product name shown in the interface")
set(SETU_PRODUCT_FULL_NAME "Speech-Enabled Tool Unification" CACHE STRING "Expanded product name")
set(SETU_SUPPORT_URL "" CACHE STRING "Support page opened from About")
set(SETU_FEEDBACK_ADDRESS "" CACHE STRING "Accessibility feedback address shown in About")
set(SETU_BUILD_ID "dev" CACHE STRING "Build identifier")
option(SETU_BUILD_TESTS "Build the test suites" ON)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.8 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 Network Svg LinguistTools)
if(UNIX AND NOT APPLE)
  find_package(Qt6 REQUIRED COMPONENTS DBus)
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(LIBSECRET REQUIRED IMPORTED_TARGET libsecret-1)
  pkg_check_modules(XCB IMPORTED_TARGET xcb xcb-shape)
  find_package(LayerShellQt QUIET)
endif()
qt_standard_project_setup(REQUIRES 6.8)
qt_policy(SET QTP0004 NEW)

add_compile_definitions(
  SETU_PRODUCT_NAME="${SETU_PRODUCT_NAME}"
  SETU_PRODUCT_FULL_NAME="${SETU_PRODUCT_FULL_NAME}"
  SETU_SUPPORT_URL="${SETU_SUPPORT_URL}"
  SETU_FEEDBACK_ADDRESS="${SETU_FEEDBACK_ADDRESS}"
  SETU_VERSION="${PROJECT_VERSION}"
  SETU_BUILD_ID="${SETU_BUILD_ID}"
)

qt_add_library(setu_core STATIC)
qt_add_qml_module(setu_core
  URI Setu.Core
  VERSION 1.0
  OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/qml/Setu/Core
  SOURCES
    src/core/daemonclient.h src/core/daemonclient.cpp
    src/core/subscription.h src/core/subscription.cpp
    src/core/settings.h src/core/settings.cpp
    src/core/platform.h src/core/platform.cpp
    src/core/brand.h src/core/brand.cpp
    src/core/launcher.h src/core/launcher.cpp
    src/core/announcer.h src/core/announcer.cpp
    src/core/iconprovider.h src/core/iconprovider.cpp
    src/core/secretstore.h src/core/secretstore.cpp
    src/core/listmodel.h src/core/listmodel.cpp
    src/core/historymodel.h src/core/historymodel.cpp
    src/core/i18n.h src/core/i18n.cpp
    src/core/setucore.h src/core/setucore.cpp
    src/core/lottie.h src/core/lottie.cpp
    src/core/profile.h src/core/profile.cpp
)
target_include_directories(setu_core PUBLIC src/core)
target_link_libraries(setu_core PUBLIC Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::Network Qt6::Svg)
if(UNIX AND NOT APPLE)
  target_link_libraries(setu_core PUBLIC Qt6::DBus ${CMAKE_DL_LIBS})
  target_include_directories(setu_core PRIVATE ${LIBSECRET_INCLUDE_DIRS})
elseif(APPLE)
  target_link_libraries(setu_core PUBLIC "-framework Security" "-framework CoreFoundation")
elseif(WIN32)
  target_link_libraries(setu_core PUBLIC advapi32 netapi32)
endif()

file(GLOB SETU_ICONS RELATIVE ${CMAKE_CURRENT_SOURCE_DIR} CONFIGURE_DEPENDS assets/icons/*.svg)
file(GLOB SETU_FONTS RELATIVE ${CMAKE_CURRENT_SOURCE_DIR} CONFIGURE_DEPENDS assets/fonts/*.ttf)
file(GLOB SETU_LOTTIE RELATIVE ${CMAKE_CURRENT_SOURCE_DIR} CONFIGURE_DEPENDS assets/lottie/*.json)
qt_add_resources(setu_core "setu_assets" PREFIX "/" BASE assets FILES ${SETU_ICONS} ${SETU_LOTTIE} assets/brand/setu.svg assets/licenses/setu.txt assets/licenses/release-notes.txt)
find_package(Python3 REQUIRED COMPONENTS Interpreter)
file(GLOB SETU_AVATAR_SOURCES CONFIGURE_DEPENDS assets/avatars/*.png)
file(GLOB SETU_BACKGROUND_SOURCES CONFIGURE_DEPENDS assets/avatars_bg/*.png)
set(SETU_LOOKS_DIR ${CMAKE_BINARY_DIR}/generated/looks)
set(SETU_LOOKS)
foreach(f ${SETU_AVATAR_SOURCES})
  get_filename_component(n ${f} NAME_WE)
  list(APPEND SETU_LOOKS ${SETU_LOOKS_DIR}/avatars/${n}.png)
endforeach()
foreach(f ${SETU_BACKGROUND_SOURCES})
  get_filename_component(n ${f} NAME_WE)
  list(APPEND SETU_LOOKS ${SETU_LOOKS_DIR}/backgrounds/${n}.jpg ${SETU_LOOKS_DIR}/thumbs/${n}.jpg)
endforeach()
add_custom_command(
  OUTPUT ${SETU_LOOKS}
  COMMAND Python3::Interpreter -I ${CMAKE_CURRENT_SOURCE_DIR}/scripts/scale-looks.py --src ${CMAKE_CURRENT_SOURCE_DIR}/assets --out ${SETU_LOOKS_DIR}
  DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/scripts/scale-looks.py ${SETU_AVATAR_SOURCES} ${SETU_BACKGROUND_SOURCES}
  COMMENT "Scaling the profile pictures"
  VERBATIM)
set_source_files_properties(${SETU_LOOKS} PROPERTIES GENERATED TRUE)
qt_add_resources(setu_core "setu_looks" PREFIX "/looks" BASE ${SETU_LOOKS_DIR} FILES ${SETU_LOOKS} OPTIONS --no-compress)
qt_add_resources(setu_core "setu_fonts" PREFIX "/" BASE assets FILES ${SETU_FONTS} OPTIONS --no-compress)

set(SETU_RUST_TARGET "" CACHE STRING "Rust target triple whose dependencies are listed in the licences")
set(SETU_LICENSE_TARGET_ARGS "")
if(SETU_RUST_TARGET)
  set(SETU_LICENSE_TARGET_ARGS --target ${SETU_RUST_TARGET})
endif()
set(SETU_LICENSES_JSON ${CMAKE_BINARY_DIR}/generated/licenses/third_party.json)
add_custom_command(
  OUTPUT ${SETU_LICENSES_JSON}
  COMMAND Python3::Interpreter -I ${CMAKE_CURRENT_SOURCE_DIR}/scripts/gen-licenses.py --out ${SETU_LICENSES_JSON} --qt-version ${Qt6_VERSION} ${SETU_LICENSE_TARGET_ARGS}
  DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/scripts/gen-licenses.py ${CMAKE_CURRENT_SOURCE_DIR}/daemon/Cargo.lock ${CMAKE_CURRENT_SOURCE_DIR}/daemon/core/assets/manifest.json
  COMMENT "Collecting third-party licenses"
  VERBATIM)
set_source_files_properties(${SETU_LICENSES_JSON} PROPERTIES GENERATED TRUE QT_RESOURCE_ALIAS licenses/third_party.json)
qt_add_resources(setu_core "setu_licenses" PREFIX "/" FILES ${SETU_LICENSES_JSON})

function(setu_qml_module target uri dir)
  file(GLOB files RELATIVE ${CMAKE_CURRENT_SOURCE_DIR} CONFIGURE_DEPENDS ${dir}/*.qml ${dir}/*/*.qml)
  foreach(f ${files})
    get_filename_component(n ${f} NAME_WE)
    if(n STREQUAL "Theme" OR n STREQUAL "Motion" OR n STREQUAL "Type" OR n STREQUAL "Tokens")
      set_source_files_properties(${f} PROPERTIES QT_QML_SINGLETON_TYPE TRUE)
    endif()
  endforeach()
  qt_add_library(${target} STATIC)
  string(REPLACE "." "/" uripath ${uri})
  qt_add_qml_module(${target} URI ${uri} VERSION 1.0 OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/qml/${uripath} QML_FILES ${files} ${ARGN})
  target_link_libraries(${target} PRIVATE setu_core)
endfunction()

setu_qml_module(setu_theme Setu.Theme qml/theme)
setu_qml_module(setu_ledger Ledger qml/controls)
target_link_libraries(setu_ledger PRIVATE setu_theme)
setu_qml_module(setu_shellqml Setu.Shell qml/overlay)
target_link_libraries(setu_shellqml PRIVATE setu_theme setu_ledger)
setu_qml_module(setu_appqml Setu.App qml/pages)
target_link_libraries(setu_appqml PRIVATE setu_theme setu_ledger)

qt_add_executable(setu-shell src/shell/main.cpp)
if(APPLE)
  enable_language(OBJCXX)
  target_sources(setu-shell PRIVATE src/shell/macwindow.mm)
  target_link_libraries(setu-shell PRIVATE "-framework AppKit")
endif()
target_link_libraries(setu-shell PRIVATE setu_core setu_coreplugin setu_theme setu_themeplugin setu_ledger setu_ledgerplugin setu_shellqml setu_shellqmlplugin Qt6::QuickControls2)
target_include_directories(setu-shell PRIVATE src/shell)
qt_add_qml_module(setu-shell URI Setu.ShellHost VERSION 1.0 NO_PLUGIN OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/qml/Setu/ShellHost SOURCES src/shell/overlayhost.h src/shell/overlayhost.cpp src/shell/hotkeys.h src/shell/hotkeys.cpp)
if(UNIX AND NOT APPLE)
  if(XCB_FOUND)
    target_link_libraries(setu-shell PRIVATE PkgConfig::XCB)
    target_compile_definitions(setu-shell PRIVATE SETU_HAVE_XCB)
  endif()
  if(LayerShellQt_FOUND)
    target_link_libraries(setu-shell PRIVATE LayerShellQt::Interface)
    target_compile_definitions(setu-shell PRIVATE SETU_HAVE_LAYERSHELL)
    target_compile_definitions(setu_core PRIVATE SETU_HAVE_LAYERSHELL)
  endif()
endif()

qt_add_executable(setu-ui src/ui/main.cpp)
target_link_libraries(setu-ui PRIVATE setu_core setu_coreplugin setu_theme setu_themeplugin setu_ledger setu_ledgerplugin setu_appqml setu_appqmlplugin Qt6::QuickControls2)

file(GLOB SETU_TS CONFIGURE_DEPENDS i18n/*.ts)
option(SETU_PSEUDO_LOCALES "Build the pseudo-locales used by the visual tests" OFF)
if(SETU_PSEUDO_LOCALES)
  execute_process(COMMAND ${Python3_EXECUTABLE} -I ${CMAKE_CURRENT_SOURCE_DIR}/scripts/i18n.py --pseudo ${CMAKE_BINARY_DIR}/i18n COMMAND_ERROR_IS_FATAL ANY)
  list(APPEND SETU_TS ${CMAKE_BINARY_DIR}/i18n/setu_qps-ploc.ts ${CMAKE_BINARY_DIR}/i18n/setu_qps-plocm.ts)
endif()
if(SETU_TS)
  qt_add_translations(setu_core TS_FILES ${SETU_TS} RESOURCE_PREFIX "/i18n" SOURCE_TARGETS setu_core setu_ledger setu_shellqml setu_appqml setu-shell setu-ui)
endif()

set_target_properties(setu-shell setu-ui PROPERTIES WIN32_EXECUTABLE ON MACOSX_BUNDLE ON)
if(WIN32)
  target_sources(setu-shell PRIVATE packaging/windows/setu.rc)
  target_sources(setu-ui PRIVATE packaging/windows/setu.rc)
endif()

install(TARGETS setu-shell setu-ui RUNTIME DESTINATION bin BUNDLE DESTINATION .)

if(SETU_BUILD_TESTS AND EXISTS "${CMAKE_SOURCE_DIR}/tests/CMakeLists.txt")
  enable_testing()
  add_subdirectory(tests)
endif()
