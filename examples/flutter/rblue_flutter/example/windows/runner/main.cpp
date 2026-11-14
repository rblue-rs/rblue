#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>

#include <cstdlib>
#include <utility>
#include <vector>

#include "flutter_window.h"
#include "utils.h"

int APIENTRY wWinMain(_In_ HINSTANCE, _In_opt_ HINSTANCE,
                      _In_ wchar_t*, _In_ int) {
  // Attach to a console when present (for example, `flutter run`) or create one
  // when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // WinRT-backed BLE requires COM initialization on the runner thread.
  const HRESULT com_result =
      ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  const bool com_initialized = SUCCEEDED(com_result);
  if (FAILED(com_result) && com_result != RPC_E_CHANGED_MODE) {
    ::MessageBox(nullptr, L"Unable to initialize COM for the rblue runner.",
                 L"rblue_flutter_example", MB_OK | MB_ICONERROR);
    return EXIT_FAILURE;
  }

  flutter::DartProject project(L"data");
  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();
  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(L"rblue Flutter example", origin, size)) {
    if (com_initialized) {
      ::CoUninitialize();
    }
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  if (com_initialized) {
    ::CoUninitialize();
  }
  return EXIT_SUCCESS;
}
