%global debug_package %{nil}

Name:           kassiber
Version:        %{kassiber_version}
Release:        %{kassiber_release}
Summary:        Local-first Bitcoin accounting desktop application
License:        MIT
URL:            https://github.com/bitcoinaustria/kassiber
Source0:        kassiber-desktop-rootfs.tar.gz
Source1:        LICENSE
BuildArch:      %{kassiber_arch}

Requires:       glibc >= 2.35
# The libraries the desktop app (crates/kassiber-desktop, Rune) loads, by soname so Fedora and
# openSUSE resolve them alike: fontconfig (linked), and the display stack winit and wgpu open at
# run time. Keep in step with crates/kassiber-desktop/packaging/linux/debian/control.in.
Requires:       libfontconfig.so.1()(64bit)
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libxkbcommon-x11.so.0()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libX11.so.6()(64bit)
Requires:       libX11-xcb.so.1()(64bit)
Requires:       libXcursor.so.1()(64bit)
Requires:       libXrandr.so.2()(64bit)
Requires:       libXi.so.6()(64bit)
Requires:       libvulkan.so.1()(64bit)
%if !0%{?suse_version}
# A Vulkan driver (wgpu has no OpenGL fallback here).
Recommends:     mesa-vulkan-drivers
%endif
Recommends:     xdg-desktop-portal
Provides:       kassiber-command
Conflicts:      kassiber-cli

%description
Kassiber is a local-first Bitcoin accounting suite. This package contains the
desktop application and its bundled command-line sidecar.

%prep
%setup -q -c -T
tar -xzf %{SOURCE0}

%build

%install
cp -a . %{buildroot}
install -Dpm 0644 %{SOURCE1} \
  %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%license %{_licensedir}/%{name}/LICENSE
%{_bindir}/kassiber
%{_bindir}/kassiber-ui
%{_prefix}/lib/Kassiber/
%{_datadir}/applications/Kassiber.desktop
%{_datadir}/icons/hicolor/*/apps/kassiber-ui.png

%changelog
* Thu Jan 01 1970 Bitcoin Austria <office@bitcoin-austria.at> - 0-1
- Package the Kassiber desktop release payload.
