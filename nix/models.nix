# Models the Home Manager module can fetch at build time instead of from the setup window.
# Each is a GGUF file on Hugging Face, kept where the app keeps the ones it downloads.
{
  parakeet = {
    repo = "handy-computer/parakeet-tdt-0.6b-v3-gguf";
    file = "parakeet-tdt-0.6b-v3-Q8_0.gguf";
    architecture = "parakeet";
    sha256 = "5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7";
  };
  gigaam = {
    repo = "handy-computer/gigaam-v3-e2e-ctc-gguf";
    file = "gigaam-v3-e2e-ctc-Q8_0.gguf";
    architecture = "gigaam";
    sha256 = "9ccce4750dc813a493d96ca15ee251712bedec15ac9a02fa3d2bd732f08ae5eb";
  };
}
