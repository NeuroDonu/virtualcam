#include "framework.h"
#include "tools.h"
#include "Registrar.h"
#include "../Common/Contract.h"
#include <mferror.h>
#include <sddl.h>
#include <shellapi.h>
#include <vector>

#define MAX_LOADSTRING 100

HINSTANCE _instance;
WCHAR _title[MAX_LOADSTRING];
WCHAR _windowClass[MAX_LOADSTRING];
wil::com_ptr_nothrow<IMFVirtualCamera> _vcam;
DWORD _vcamCookie;
HANDLE _frameMapping = nullptr;
void* _frameView = nullptr;
HANDLE _readyEvent = nullptr;
HANDLE _ownerMutex = nullptr;
HANDLE _stopEvent = nullptr;
bool _ownsRegistrarMutex = false;

constexpr wchar_t RegistrarOwnerMutexName[] = L"Global\\noperson_vcam_registrar_owner_v1";
constexpr wchar_t RegistrarStopEventName[] = L"Global\\noperson_vcam_registrar_stop_v1";
constexpr wchar_t StartupAckPrefix[] = L"Local\\noperson_vcam_startup_";

struct RequestedFormat
{
	std::uint32_t width = vcam::contract::DefaultWidth;
	std::uint32_t height = vcam::contract::DefaultHeight;
	std::uint32_t fpsNumerator = vcam::contract::DefaultFpsNumerator;
	std::uint32_t fpsDenominator = vcam::contract::DefaultFpsDenominator;
};

ATOM MyRegisterClass(HINSTANCE hInstance);
HWND InitInstance(HINSTANCE, int);
LRESULT CALLBACK WndProc(HWND, UINT, WPARAM, LPARAM);
INT_PTR CALLBACK About(HWND, UINT, WPARAM, LPARAM);
HRESULT RegisterVirtualCamera();
HRESULT UnregisterVirtualCamera();
HRESULT ReplaceOwnedVirtualCamera();
HRESULT CreateSharedFrameMapping(const RequestedFormat& format);
HRESULT SignalStartupAck(const std::wstring& startupAckName);
HRESULT ParseArguments(bool* headless, bool* replace, bool* remove,
	RequestedFormat* format, std::wstring* startupAckName);
HRESULT AcquireRegistrarOwnership(bool replace);
void ReleaseRegistrarOwnership();

int APIENTRY wWinMain(_In_ HINSTANCE hInstance, _In_opt_ HINSTANCE hPrevInstance, _In_ LPWSTR lpCmdLine, _In_ int nCmdShow)
{
	UNREFERENCED_PARAMETER(hPrevInstance);
	UNREFERENCED_PARAMETER(lpCmdLine);
	bool headless = false;
	bool replace = false;
	bool remove = false;
	HRESULT exitResult = S_OK;
	RequestedFormat requestedFormat{};
	std::wstring startupAckName;
	if (FAILED(ParseArguments(
		&headless, &replace, &remove, &requestedFormat, &startupAckName)))
	{
		return ERROR_INVALID_PARAMETER;
	}

	// set tracing & CRT leak tracking
	WinTraceRegister();
	WINTRACE(L"WinMain starting '%s'", GetCommandLineW());
	_CrtSetReportMode(_CRT_WARN, _CRTDBG_MODE_DEBUG);

	wil::SetResultLoggingCallback([](wil::FailureInfo const& failure) noexcept
		{
			wchar_t str[2048];
			if (SUCCEEDED(wil::GetFailureLogString(str, _countof(str), failure)))
			{
				WinTrace(2, 0, str); // 2 => error
#ifndef _DEBUG
				TaskDialog(nullptr, nullptr, _title, L"A fatal error has occured. Press OK to terminate.", str, TDCBF_OK_BUTTON, TD_WARNING_ICON, nullptr);
#endif
			}
		});

	LoadStringW(hInstance, IDS_APP_TITLE, _title, MAX_LOADSTRING);
	lstrcpyW(_title, vcam::contract::FriendlyName);
	LoadStringW(hInstance, IDC_VCAM, _windowClass, MAX_LOADSTRING);
	MyRegisterClass(hInstance);
	auto hwnd = InitInstance(hInstance, headless ? SW_HIDE : nCmdShow);
	if (hwnd)
	{
		winrt::init_apartment();
		if (SUCCEEDED(MFStartup(MF_VERSION)))
		{
			TASKDIALOGCONFIG config{};
			config.cbSize = sizeof(TASKDIALOGCONFIG);
			config.hInstance = hInstance;
			config.hwndParent = hwnd;
			config.pszWindowTitle = _title;
			config.dwCommonButtons = TDCBF_CLOSE_BUTTON;
			auto hr = AcquireRegistrarOwnership(replace || remove);
			if (SUCCEEDED(hr) && (replace || remove))
			{
				hr = ReplaceOwnedVirtualCamera();
			}
			if (SUCCEEDED(hr) && !remove)
			{
				hr = CreateSharedFrameMapping(requestedFormat);
			}
			if (SUCCEEDED(hr) && !remove)
			{
				hr = RegisterVirtualCamera();
			}
			if (SUCCEEDED(hr) && !remove)
			{
				hr = SignalStartupAck(startupAckName);
			}
			exitResult = hr;
			if (SUCCEEDED(hr) && !remove)
			{
				if (headless)
				{
					// The replacement helper signals only this product-owned event.
					// No process enumeration or Windows service control is involved.
					WaitForSingleObject(_stopEvent, INFINITE);
				}
				else
				{
					config.pszMainInstruction = L"VCam is running.";
					config.pszContent = L"Camera clients can now open VCam. Close this window to unregister the session camera.";
					config.pszMainIcon = TD_INFORMATION_ICON;
					TaskDialogIndirect(&config, nullptr, nullptr, nullptr);
				}

				//auto accelerators = LoadAccelerators(hInstance, MAKEINTRESOURCE(IDC_VCAM));
				//MSG msg;
				//while (GetMessage(&msg, nullptr, 0, 0))
				//{
				//	if (!TranslateAccelerator(msg.hwnd, accelerators, &msg))
				//	{
				//		TranslateMessage(&msg);
				//		DispatchMessage(&msg);
				//	}
				//}

				UnregisterVirtualCamera();
			}
			else if (FAILED(hr))
			{
				config.pszMainInstruction = replace || remove
					? L"VCam could not replace its existing registration."
					: L"VCam could not start. Register VCamSource.dll first.";
				wchar_t text[1024];
				wchar_t errorText[256];
				FormatMessage(FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS, nullptr, hr, 0, errorText, _countof(errorText), nullptr);
				if (replace || remove)
				{
					wsprintf(text, L"Error 0x%08X (%u): %s\n\nClose camera clients and retry. If Windows still retains the previous camera session, restart Windows and run the command again.", hr, hr, errorText);
				}
				else
				{
					wsprintf(text, L"Error 0x%08X (%u): %s", hr, hr, errorText);
				}
				config.pszContent = text;

				config.pszMainIcon = TD_ERROR_ICON;
				TaskDialogIndirect(&config, nullptr, nullptr, nullptr);
			}

			_vcam.reset();
			MFShutdown();
		}
	}
	if (_frameMapping)
	{
		if (_frameView)
		{
			UnmapViewOfFile(_frameView);
			_frameView = nullptr;
		}
		CloseHandle(_frameMapping);
		_frameMapping = nullptr;
	}
	if (_readyEvent)
	{
		CloseHandle(_readyEvent);
		_readyEvent = nullptr;
	}
	ReleaseRegistrarOwnership();

	// cleanup & CRT leak checks
	_CrtDumpMemoryLeaks();
	WINTRACE(L"WinMain exiting '%s'", GetCommandLineW());
	WinTraceUnregister();
	return SUCCEEDED(exitResult) ? ERROR_SUCCESS : HRESULT_CODE(exitResult);
}

HRESULT ParseArguments(bool* headless, bool* replace, bool* remove,
	RequestedFormat* format, std::wstring* startupAckName)
{
	RETURN_HR_IF_NULL(E_POINTER, headless);
	RETURN_HR_IF_NULL(E_POINTER, replace);
	RETURN_HR_IF_NULL(E_POINTER, remove);
	RETURN_HR_IF_NULL(E_POINTER, format);
	RETURN_HR_IF_NULL(E_POINTER, startupAckName);
	int argc = 0;
	auto argv = CommandLineToArgvW(GetCommandLineW(), &argc);
	RETURN_LAST_ERROR_IF_NULL(argv);

	auto parseU32 = [](const wchar_t* text, std::uint32_t* value) -> bool
	{
		if (!text || !*text || !value) return false;
		wchar_t* end = nullptr;
		const auto parsed = wcstoull(text, &end, 10);
		if (!end || *end != L'\0' || parsed > UINT32_MAX) return false;
		*value = static_cast<std::uint32_t>(parsed);
		return true;
	};

	HRESULT result = S_OK;
	for (int index = 1; index < argc; ++index)
	{
		if (_wcsicmp(argv[index], L"/headless") == 0)
		{
			*headless = true;
		}
		else if (_wcsicmp(argv[index], L"/replace") == 0)
		{
			*replace = true;
			if (index + 4 >= argc
				|| !parseU32(argv[index + 1], &format->width)
				|| !parseU32(argv[index + 2], &format->height)
				|| !parseU32(argv[index + 3], &format->fpsNumerator)
				|| !parseU32(argv[index + 4], &format->fpsDenominator))
			{
				result = E_INVALIDARG;
				break;
			}
			index += 4;
		}
		else if (_wcsicmp(argv[index], L"/remove") == 0)
		{
			*remove = true;
			*headless = true;
		}
		else if (_wcsicmp(argv[index], L"/ack") == 0)
		{
			if (index + 1 >= argc || !argv[index + 1][0])
			{
				result = E_INVALIDARG;
				break;
			}
			*startupAckName = argv[++index];
		}
		else
		{
			result = E_INVALIDARG;
			break;
		}
	}
	LocalFree(argv);
	if (FAILED(result)) return result;
	auto validStartupAck = [&]() -> bool
	{
		if (startupAckName->empty()) return true;
		constexpr size_t prefixLength = _countof(StartupAckPrefix) - 1;
		constexpr size_t suffixLength = 8 + 1 + 16;
		if (startupAckName->size() != prefixLength + suffixLength
			|| startupAckName->compare(0, prefixLength, StartupAckPrefix) != 0
			|| (*startupAckName)[prefixLength + 8] != L'_')
		{
			return false;
		}
		for (size_t offset = 0; offset < suffixLength; ++offset)
		{
			if (offset == 8) continue;
			const auto value = (*startupAckName)[prefixLength + offset];
			if (!((value >= L'0' && value <= L'9')
				|| (value >= L'a' && value <= L'f')))
			{
				return false;
			}
		}
		return true;
	};
	if ((*replace && *remove)
		|| (!startupAckName->empty() && !*replace)
		|| !validStartupAck()
		|| !format->width || !format->height
		|| (format->width & 1) || (format->height & 1)
		|| format->width > vcam::contract::MaxWidth
		|| format->height > vcam::contract::MaxHeight
		|| !format->fpsNumerator || !format->fpsDenominator
		|| format->fpsNumerator > 1000ULL * format->fpsDenominator)
	{
		return E_INVALIDARG;
	}
	return S_OK;
}

HRESULT AcquireRegistrarOwnership(bool replace)
{
	_ownerMutex = CreateMutexW(nullptr, FALSE, RegistrarOwnerMutexName);
	RETURN_LAST_ERROR_IF_NULL(_ownerMutex);
	const bool previousOwnerUsesProtocol = GetLastError() == ERROR_ALREADY_EXISTS;

	if (replace && previousOwnerUsesProtocol)
	{
		const auto previousStop = OpenEventW(EVENT_MODIFY_STATE, FALSE, RegistrarStopEventName);
		if (!previousStop)
		{
			return HRESULT_FROM_WIN32(ERROR_REVISION_MISMATCH);
		}
		const auto signaled = SetEvent(previousStop);
		const auto signalError = signaled ? ERROR_SUCCESS : GetLastError();
		CloseHandle(previousStop);
		RETURN_HR_IF(HRESULT_FROM_WIN32(signalError), !signaled);
	}

	const auto waitResult = WaitForSingleObject(_ownerMutex, replace ? 10000 : 0);
	if (waitResult != WAIT_OBJECT_0 && waitResult != WAIT_ABANDONED)
	{
		return waitResult == WAIT_TIMEOUT
			? HRESULT_FROM_WIN32(ERROR_TIMEOUT)
			: HRESULT_FROM_WIN32(GetLastError());
	}
	_ownsRegistrarMutex = true;

	_stopEvent = CreateEventW(nullptr, TRUE, FALSE, RegistrarStopEventName);
	RETURN_LAST_ERROR_IF_NULL(_stopEvent);
	RETURN_LAST_ERROR_IF(!ResetEvent(_stopEvent));
	return S_OK;
}

void ReleaseRegistrarOwnership()
{
	if (_stopEvent)
	{
		CloseHandle(_stopEvent);
		_stopEvent = nullptr;
	}
	if (_ownerMutex)
	{
		if (_ownsRegistrarMutex)
		{
			ReleaseMutex(_ownerMutex);
			_ownsRegistrarMutex = false;
		}
		CloseHandle(_ownerMutex);
		_ownerMutex = nullptr;
	}
}

HRESULT ReplaceOwnedVirtualCamera()
{
	auto clsid = GUID_ToStringW(vcam::contract::SourceClsid);
	wil::com_ptr_nothrow<IMFVirtualCamera> ownedCamera;
	RETURN_IF_FAILED_MSG(MFCreateVirtualCamera(
		MFVirtualCameraType_SoftwareCameraSource,
		MFVirtualCameraLifetime_Session,
		MFVirtualCameraAccess_CurrentUser,
		vcam::contract::FriendlyName,
		clsid.c_str(),
		nullptr,
		0,
		&ownedCamera),
		"Failed to open the owned VCam registration");
	const auto removeResult = ownedCamera->Remove();
	if (removeResult == HRESULT_FROM_WIN32(ERROR_NOT_FOUND) || removeResult == MF_E_NOT_FOUND)
	{
		return S_OK;
	}
	return removeResult;
}

HRESULT CreateSharedFrameMapping(const RequestedFormat& format)
{
	HANDLE processToken = nullptr;
	LPWSTR tokenUserSid = nullptr;
	PSECURITY_DESCRIPTOR descriptor = nullptr;
	HANDLE mapping = nullptr;
	void* view = nullptr;

	const auto result = [&]() -> HRESULT
	{
		if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &processToken))
		{
			return HRESULT_FROM_WIN32(GetLastError());
		}

		DWORD tokenBytes = 0;
		if (GetTokenInformation(processToken, TokenUser, nullptr, 0, &tokenBytes))
		{
			return HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
		}
		const auto tokenProbeError = GetLastError();
		if (tokenProbeError != ERROR_INSUFFICIENT_BUFFER)
		{
			return HRESULT_FROM_WIN32(tokenProbeError);
		}
		if (tokenBytes < sizeof(TOKEN_USER))
		{
			return HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
		}
		std::vector<std::uint8_t> tokenBuffer(tokenBytes);
		if (!GetTokenInformation(
			processToken, TokenUser, tokenBuffer.data(), tokenBytes, &tokenBytes))
		{
			return HRESULT_FROM_WIN32(GetLastError());
		}
		const auto* tokenUser = reinterpret_cast<const TOKEN_USER*>(tokenBuffer.data());
		if (!IsValidSid(tokenUser->User.Sid))
		{
			return HRESULT_FROM_WIN32(ERROR_INVALID_SID);
		}
		if (!ConvertSidToStringSidW(tokenUser->User.Sid, &tokenUserSid))
		{
			return HRESULT_FROM_WIN32(GetLastError());
		}

		std::wstring sddl = L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;";
		sddl += tokenUserSid;
		sddl += L")(A;;GR;;;LS)";
		if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
			sddl.c_str(), SDDL_REVISION_1, &descriptor, nullptr))
		{
			return HRESULT_FROM_WIN32(GetLastError());
		}

		SECURITY_ATTRIBUTES attributes{};
		attributes.nLength = sizeof(attributes);
		attributes.lpSecurityDescriptor = descriptor;
		const DWORD mappingBytes = sizeof(vcam::contract::SharedHeader)
			+ vcam::contract::MaxFrameBytes;
		mapping = CreateFileMappingW(
			INVALID_HANDLE_VALUE,
			&attributes,
			PAGE_READWRITE,
			0,
			mappingBytes,
			vcam::contract::SharedMemoryName);
		const auto mappingCreateError = GetLastError();
		if (!mapping)
		{
			return HRESULT_FROM_WIN32(mappingCreateError);
		}
		if (mappingCreateError == ERROR_ALREADY_EXISTS)
		{
			return HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS);
		}
		if (!SetKernelObjectSecurity(
			mapping,
			DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
			descriptor))
		{
			return HRESULT_FROM_WIN32(GetLastError());
		}

		view = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, mappingBytes);
		if (!view)
		{
			return HRESULT_FROM_WIN32(GetLastError());
		}

		auto* header = static_cast<vcam::contract::SharedHeader*>(view);
		InterlockedExchange(reinterpret_cast<volatile LONG*>(&header->ready), 0);
		InterlockedExchange(reinterpret_cast<volatile LONG*>(&header->seq), 1);
		MemoryBarrier();
		header->magic = vcam::contract::SharedMemoryMagic;
		header->version = vcam::contract::HeaderVersion;
		header->headerBytes = sizeof(*header);
		header->width = format.width;
		header->height = format.height;
		header->fpsNumerator = format.fpsNumerator;
		header->fpsDenominator = format.fpsDenominator;
		header->frameBytes = format.width * format.height * 3 / 2;
		header->reserved[0] = 0;
		header->reserved[1] = 0;
		MemoryBarrier();
		InterlockedExchange(reinterpret_cast<volatile LONG*>(&header->seq), 2);
		return S_OK;
	}();

	if (descriptor) LocalFree(descriptor);
	if (tokenUserSid) LocalFree(tokenUserSid);
	if (processToken) CloseHandle(processToken);
	if (FAILED(result))
	{
		if (view) UnmapViewOfFile(view);
		if (mapping) CloseHandle(mapping);
		return result;
	}

	_frameMapping = mapping;
	_frameView = view;
	WINTRACE(L"Shared frame mapping '%s' is ready", vcam::contract::SharedMemoryName);
	return S_OK;
}

HRESULT SignalStartupAck(const std::wstring& startupAckName)
{
	if (startupAckName.empty()) return S_OK;
	const auto startupAck = OpenEventW(
		EVENT_MODIFY_STATE, FALSE, startupAckName.c_str());
	RETURN_LAST_ERROR_IF_NULL(startupAck);
	const auto signaled = SetEvent(startupAck);
	const auto error = signaled ? ERROR_SUCCESS : GetLastError();
	CloseHandle(startupAck);
	return HRESULT_FROM_WIN32(error);
}

HRESULT RegisterVirtualCamera()
{
	auto clsid = GUID_ToStringW(vcam::contract::SourceClsid);
	RETURN_IF_FAILED_MSG(MFCreateVirtualCamera(
		MFVirtualCameraType_SoftwareCameraSource,
		MFVirtualCameraLifetime_Session,
		MFVirtualCameraAccess_CurrentUser,
		_title,
		clsid.c_str(),
		nullptr,
		0,
		&_vcam),
		"Failed to create virtual camera");

	WINTRACE(L"RegisterVirtualCamera '%s' ok", clsid.c_str());
	RETURN_IF_FAILED_MSG(_vcam->Start(nullptr), "Cannot start VCam");
	_readyEvent = CreateEventW(nullptr, TRUE, TRUE, vcam::contract::RegistrarReadyEventName);
	RETURN_LAST_ERROR_IF_NULL(_readyEvent);
	WINTRACE(L"VCam was started");
	return S_OK;
}

HRESULT UnregisterVirtualCamera()
{
	if (!_vcam)
		return S_OK;

	// NOTE: we don't call Shutdown or this will cause 2 Shutdown calls to the media source and will prevent proper removing
	//auto hr = _vcam->Shutdown();
	//WINTRACE(L"Shutdown VCam hr:0x%08X", hr);

	auto hr = _vcam->Remove();
	WINTRACE(L"Remove VCam hr:0x%08X", hr);
	return S_OK;
}

ATOM MyRegisterClass(HINSTANCE instance)
{
	WNDCLASSEXW wcex{};
	wcex.cbSize = sizeof(WNDCLASSEX);
	wcex.style = CS_HREDRAW | CS_VREDRAW;
	wcex.lpfnWndProc = WndProc;
	wcex.hInstance = instance;
	wcex.hIcon = LoadIcon(instance, MAKEINTRESOURCE(IDI_VCAM));
	wcex.hCursor = LoadCursor(nullptr, IDC_ARROW);
	wcex.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
	wcex.lpszMenuName = MAKEINTRESOURCEW(IDC_VCAM);
	wcex.lpszClassName = _windowClass;
	wcex.hIconSm = LoadIcon(wcex.hInstance, MAKEINTRESOURCE(IDI_SMALL));
	return RegisterClassExW(&wcex);
}

HWND InitInstance(HINSTANCE instance, int cmd)
{
	_instance = instance;
	auto hwnd = CreateWindowW(_windowClass, _title, WS_OVERLAPPEDWINDOW, 0, 0, 600, 400, nullptr, nullptr, instance, nullptr);
	if (!hwnd)
		return nullptr;

	CenterWindow(hwnd);
	ShowWindow(hwnd, cmd);
	UpdateWindow(hwnd);
	return hwnd;
}

LRESULT CALLBACK WndProc(HWND hwnd, UINT message, WPARAM wParam, LPARAM lParam)
{
	//#if _DEBUG
	//	if (message != WM_NCMOUSEMOVE && message != WM_MOUSEMOVE && message != WM_SETCURSOR && message != WM_NCHITTEST && message != WM_MOUSELEAVE &&
	//		message != WM_GETICON && message != WM_PAINT)
	//	{
	//		if (message == 147 || message == 148)
	//		{
	//			WINTRACE("msg:%u 0x%08X (%s)", message, message, WM_ToString(message).c_str());
	//		}
	//	}
	//#endif

	switch (message)
	{
	case WM_COMMAND:
	{
		auto wmId = LOWORD(wParam);
		switch (wmId)
		{
		case IDM_ABOUT:
			DialogBox(_instance, MAKEINTRESOURCE(IDD_ABOUTBOX), hwnd, About);
			break;

		case IDM_EXIT:
			DestroyWindow(hwnd);
			break;
		default:
			return DefWindowProc(hwnd, message, wParam, lParam);
		}
	}
	break;

	case WM_PAINT:
	{
		PAINTSTRUCT ps;
		auto hdc = BeginPaint(hwnd, &ps);
		// TODO: Add any drawing code that uses hdc here...
		EndPaint(hwnd, &ps);
	}
	break;

	case WM_DESTROY:
		PostQuitMessage(0);
		break;

	default:
		return DefWindowProc(hwnd, message, wParam, lParam);
	}
	return 0;
}

INT_PTR CALLBACK About(HWND hwnd, UINT message, WPARAM wParam, LPARAM lParam)
{
	UNREFERENCED_PARAMETER(lParam);
	switch (message)
	{
	case WM_INITDIALOG:
		return (INT_PTR)TRUE;

	case WM_COMMAND:
		if (LOWORD(wParam) == IDOK || LOWORD(wParam) == IDCANCEL)
		{
			EndDialog(hwnd, LOWORD(wParam));
			return (INT_PTR)TRUE;
		}
		break;
	}
	return (INT_PTR)FALSE;
}
