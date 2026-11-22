Name:           geectl-e2e-hello
Version:        1.0
Release:        1
Summary:        End-to-end fixture for the local-build CI job
License:        MIT
BuildArch:      noarch

%description
End-to-end fixture for the local-build CI job.

%prep
%build
%install
mkdir -p %{buildroot}/usr/share/doc/%{name}
printf 'geectl local-build e2e\n' > %{buildroot}/usr/share/doc/%{name}/README

%files
/usr/share/doc/%{name}/README
