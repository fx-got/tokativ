# The model files the daemon downloads on first run, for prefetching them at build time instead.
# Keep in step with crates/engine/src/models.rs.
{
  parakeet = [
    {
      url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/encoder-model.int8.onnx";
      name = "encoder-model.int8.onnx";
      sha256 = "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09";
    }
    {
      url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/decoder_joint-model.int8.onnx";
      name = "decoder_joint-model.int8.onnx";
      sha256 = "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70";
    }
    {
      url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx";
      name = "nemo128.onnx";
      sha256 = "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f";
    }
    {
      url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/vocab.txt";
      name = "vocab.txt";
      sha256 = "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d";
    }
  ];
  gigaam = [
    {
      url = "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc.int8.onnx";
      name = "model.int8.onnx";
      sha256 = "2e3fcb7a7b66030336fd10c2fcfb033bd1dc7e1bf238fe5cfd83b1d0cfc9d28e";
    }
    {
      url = "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc_vocab.txt";
      name = "vocab.txt";
      sha256 = "142de7570b3de5b3035ce111a89c228e80e6085273731d944093ddf24fa539cd";
    }
  ];
}
