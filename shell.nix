with import <nixpkgs> {};

mkShell {
  buildInputs = [
    rustc
    cargo
    gcc
    pkg-config
    wayland
    libxkbcommon
    mesa
    libglvnd
    libdrm
    libgbm
  ];
  shellHook = ''
    if [ -n "$LD_LIBRARY_PATH" ]; then
      export LD_LIBRARY_PATH="${mesa}/lib:${libglvnd}/lib:${libdrm}/lib:${libgbm}/lib:${wayland}/lib:${libxkbcommon}/lib:$LD_LIBRARY_PATH"
    else
      export LD_LIBRARY_PATH="${mesa}/lib:${libglvnd}/lib:${libdrm}/lib:${libgbm}/lib:${wayland}/lib:${libxkbcommon}/lib"
    fi
    if [ -d /run/opengl-driver/lib ]; then
      export LD_LIBRARY_PATH="/run/opengl-driver/lib:$LD_LIBRARY_PATH"
    fi
  '';
}
