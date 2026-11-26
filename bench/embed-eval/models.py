"""The candidate embedding models of VFS-29b and one way to run each (VFS-29 R16).

Every model turns a list of texts into L2-normalised float32 vectors. Transformer models run
through ONNX Runtime on the CPU with the file their card names for CPU use; static models are
a token-vector lookup and a mean. Shared by bench.py (quality) and time_models.py (cost).
"""

import os

import numpy as np

# Transformer inputs are cut to this many tokens: multilingual-e5-small's limit, and about
# one VFS chunk (2,000 characters) for the others; longer contexts cost more CPU per chunk.
MAX_TOKENS = 512

MODELS = {
    "bekko-static-512": {
        "repo": "hotchpotch/bekko-static-embedding-v1",
        "files": ["onnx/model_int8_dim512.onnx", "tokenizer.json"],
        "kind": "static-onnx",
        "dim": 512,
        "licence": "MIT",
    },
    "bekko-static-1024": {
        "repo": "hotchpotch/bekko-static-embedding-v1",
        "files": ["onnx/model_int8_dim1024.onnx", "tokenizer.json"],
        "kind": "static-onnx",
        "dim": 1024,
        "licence": "MIT",
    },
    "potion-multilingual-128M": {
        "repo": "minishlab/potion-multilingual-128M",
        "files": ["model.safetensors", "tokenizer.json", "config.json"],
        "kind": "model2vec",
        "dim": 256,
        "licence": "MIT",
    },
    "granite-97m-r2-int8": {
        "repo": "ibm-granite/granite-embedding-97m-multilingual-r2",
        "files": ["onnx/model_quint8_avx2.onnx", "tokenizer.json"],
        "kind": "transformer",
        "pooling": "cls",
        "dim": 384,
        "licence": "Apache-2.0",
    },
    "e5-small-int8": {
        "repo": "intfloat/multilingual-e5-small",
        "files": ["onnx/model_qint8_avx512_vnni.onnx", "tokenizer.json"],
        "kind": "transformer",
        "pooling": "mean",
        "prefix": ("query: ", "passage: "),
        "token_type_ids": True,
        "dim": 384,
        "licence": "MIT",
    },
    "bekko-a8m": {
        "repo": "hotchpotch/bekko-embedding-v1-a8m",
        "files": ["onnx/model.onnx", "tokenizer.json"],
        "kind": "transformer",
        "pooling": "mean",
        "dim": 384,
        "licence": "MIT",
    },
}


def fetch(name):
    """Local paths of the model's files, downloading them on first use; and their total size."""
    from huggingface_hub import hf_hub_download

    spec = MODELS[name]
    paths = [hf_hub_download(spec["repo"], f) for f in spec["files"]]
    return paths, sum(os.path.getsize(p) for p in paths)


def _normalise(v):
    v = np.asarray(v, dtype=np.float32)
    return v / np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-12)


class Encoder:
    def __init__(self, name, threads=1):
        import onnxruntime as ort
        from tokenizers import Tokenizer

        self.name = name
        self.spec = spec = MODELS[name]
        paths, self.download_bytes = fetch(name)
        if spec["kind"] == "model2vec":
            from model2vec import StaticModel

            self.m2v = StaticModel.from_pretrained(os.path.dirname(paths[0]))
            return
        opts = ort.SessionOptions()
        opts.intra_op_num_threads = threads
        opts.inter_op_num_threads = 1
        self.session = ort.InferenceSession(paths[0], opts, providers=["CPUExecutionProvider"])
        self.tok = Tokenizer.from_file(paths[1])
        self.tok.no_padding()
        if spec["kind"] == "transformer":
            self.tok.enable_truncation(MAX_TOKENS)
        else:
            self.tok.no_truncation()

    def tokens(self, text, query=False):
        """Token count of one text as the model sees it (after any truncation)."""
        if self.spec["kind"] == "model2vec":
            return len(self.m2v.tokenizer.encode(text, add_special_tokens=False).ids)
        return len(self._encode([text], query)[0])

    def _encode(self, texts, query):
        prefix = self.spec.get("prefix")
        if prefix:
            texts = [(prefix[0] if query else prefix[1]) + t for t in texts]
        special = self.spec["kind"] == "transformer"
        return [e.ids for e in self.tok.encode_batch(texts, add_special_tokens=special)]

    def embed(self, texts, query=False, batch=1):
        if self.spec["kind"] == "model2vec":
            return _normalise(self.m2v.encode(texts, batch_size=max(batch, 1024)))
        out = []
        for i in range(0, len(texts), batch):
            ids = self._encode(texts[i:i + batch], query)
            width = max(1, max(len(x) for x in ids))
            input_ids = np.zeros((len(ids), width), dtype=np.int64)
            mask = np.zeros_like(input_ids)
            for r, x in enumerate(ids):
                input_ids[r, :len(x)] = x
                mask[r, :len(x)] = 1
            feed = {"input_ids": input_ids, "attention_mask": mask}
            if self.spec.get("token_type_ids"):
                feed["token_type_ids"] = np.zeros_like(input_ids)
            y = self.session.run(None, feed)[0]
            if self.spec["kind"] == "static-onnx":
                out.append(y)
            elif self.spec["pooling"] == "cls":
                out.append(y[:, 0])
            else:
                m = mask[:, :, None].astype(np.float32)
                out.append((y * m).sum(1) / np.maximum(m.sum(1), 1e-9))
        return _normalise(np.concatenate(out))
