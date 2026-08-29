"""Native-mode SDK response compatibility tests."""

import json

import pytest
from oryn.client import OrynClient


class _NativeTransport:
    async def send(self, command: str) -> str:
        assert command == "observe"
        return json.dumps(
            {
                "kind": "observation",
                "observation": {
                    "contract_version": 2,
                    "revision": 4,
                    "execution_domain": "native",
                    "page": {
                        "url": "https://example.test/",
                        "title": "Example",
                        "document_generation": 2,
                    },
                    "nodes": [
                        {
                            "alias": 7,
                            "role": "button",
                            "name": "Continue",
                            "states": ["visible"],
                            "actions": ["click"],
                            "semantic_ref": {
                                "document_generation": 2,
                                "node": 9,
                            },
                            "document_order": 8,
                            "selector": "#continue",
                            "content_trust": "page_authored",
                            "provenance": {
                                "kind": "heuristic",
                                "confidence": 0.7,
                            },
                        }
                    ],
                    "capabilities": [
                        {
                            "capability": "rendered_layout_and_paint",
                            "support": "unsupported",
                            "alternatives": ["chromium"],
                            "handoff_lossy": True,
                        }
                    ],
                    "diagnostics": ["fixture diagnostic"],
                },
            }
        )


class _NativeActionTransport:
    async def send(self, command: str) -> str:
        assert command == 'click "Continue"'
        return json.dumps(
            {
                "kind": "action",
                "result": {
                    "contract_version": 2,
                    "action_id": 3,
                    "revision_before": 4,
                    "revision_after": 5,
                    "delta": {
                        "contract_version": 2,
                        "from_revision": 4,
                        "to_revision": 5,
                        "upserted": [],
                        "removed": [],
                    },
                    "effects": [
                        {"kind": "event", "event_type": "click"},
                        {"kind": "dom_mutation", "summary": "changed"},
                    ],
                    "diagnostics": [],
                    "execution_domain": "native",
                },
            }
        )


class _CompatibilityObservationTransport:
    async def send(self, command: str) -> str:
        assert command == "scan"
        return '@ https://example.test/ "Café"\n[1] button "Continué"'


class _NativeGeometryTransport:
    async def send(self, command: str) -> str:
        assert command == "observe --positions --full"
        return json.dumps(
            {
                "kind": "geometry_observation",
                "observation": {
                    "contract_version": 4,
                    "revision": 8,
                    "style_revision": 2,
                    "layout_revision": 3,
                    "execution_domain": "native",
                    "page": {
                        "url": "https://layout.test/",
                        "title": "Layout",
                        "document_generation": 1,
                    },
                    "viewport": {
                        "width": 320.0,
                        "height": 200.0,
                        "device_pixel_ratio": 1.0,
                    },
                    "scroll": {
                        "x": 0.0,
                        "y": 40.0,
                        "content_width": 320.0,
                        "content_height": 900.0,
                    },
                    "nodes": [
                        {
                            "alias": 1,
                            "role": "button",
                            "name": "Continue",
                            "states": ["visible"],
                            "actions": ["click"],
                            "semantic_ref": {
                                "page_id": 1,
                                "document_generation": 1,
                                "node": 2,
                            },
                            "document_order": 2,
                            "geometry": {
                                "availability": "available",
                                "in_viewport": True,
                                "interactable": True,
                            },
                        }
                    ],
                },
            }
        )


@pytest.mark.asyncio
async def test_native_observation_uses_existing_json_oil_contract():
    client = OrynClient(mode="native")
    client._transport = _NativeTransport()

    observation = await client.observe()

    assert observation.url == "https://example.test/"
    assert observation.title == "Example"
    assert observation.elements[0]["id"] == 7
    assert observation.elements[0]["actions"] == ["click"]
    assert observation.contract_version == 2
    assert observation.revision == 4
    assert observation.document_generation == 2
    assert observation.elements[0]["selector"] == "#continue"
    assert observation.elements[0]["content_trust"] == "page_authored"
    assert observation.elements[0]["provenance"]["kind"] == "heuristic"
    assert observation.capabilities[0].capability == "rendered_layout_and_paint"
    assert observation.diagnostics == ["fixture diagnostic"]
    assert observation.byte_count == len(observation.raw.encode("utf-8"))


@pytest.mark.asyncio
async def test_native_action_is_classified_from_effects_and_delta():
    client = OrynClient(mode="native")
    client._transport = _NativeActionTransport()

    result = await client.execute_typed('click "Continue"')

    assert result.success is True
    assert result.accepted is True
    assert result.classification == "state_changed"
    assert [effect.kind for effect in result.effects] == ["event", "dom_mutation"]
    assert result.delta is not None
    assert result.delta.from_revision == 4
    assert result.revision_before == 4
    assert result.revision_after == 5


@pytest.mark.asyncio
async def test_compatibility_observation_measures_exact_utf8_bytes():
    client = OrynClient(mode="headless")
    client._transport = _CompatibilityObservationTransport()

    observation = await client.observe()

    assert observation.byte_count == len(observation.raw.encode("utf-8"))
    assert observation.byte_count > len(observation.raw)


@pytest.mark.asyncio
async def test_native_v4_geometry_observation_is_typed_without_losing_nodes():
    client = OrynClient(mode="native")
    client._transport = _NativeGeometryTransport()

    observation = await client.observe(positions=True, full=True)

    assert observation.contract_version == 4
    assert observation.style_revision == 2
    assert observation.layout_revision == 3
    assert observation.viewport.width == 320.0
    assert observation.scroll.y == 40.0
    assert observation.elements[0]["geometry"]["interactable"] is True
    assert observation.elements[0]["content_trust"] == "page_authored"
