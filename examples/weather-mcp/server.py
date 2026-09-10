#!/usr/bin/env python3
"""Weather MCP server for liteclaw.

A minimal MCP server (stdio JSON-RPC) that exposes one tool:
`get_forecast(city)` — returns the current weather via the free wttr.in API
(no API key required).

Wire format (one JSON-RPC message per line on stdin/stdout):
  initialize  → handshake
  tools/list  → describe available tools
  tools/call  → execute a tool

Config in ~/.liteclaw/mcp.json:
  {
    "servers": {
      "weather": {
        "command": "python3",
        "args": ["/absolute/path/to/examples/weather-mcp/server.py"]
      }
    }
  }

Then in liteclaw chat: "北京今天天气怎么样?"
The model will call mcp__weather__get_forecast and return the result.
"""

import json
import sys
import urllib.request
import urllib.error


def get_forecast(city: str) -> str:
    """Fetch weather for `city` from wttr.in (free, no key)."""
    url = f"https://wttr.in/{urllib.parse.quote(city)}?format=j1"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "liteclaw/0.1"})
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode("utf-8"))
    except urllib.error.URLError as e:
        return f"天气查询失败(网络错误): {e.reason}"
    except Exception as e:
        return f"天气查询失败: {e}"

    # wttr.in JSON format: data["current_condition"][0] has the current weather.
    current = data.get("current_condition", [{}])[0]
    area = data.get("nearest_area", [{}])[0]
    area_name = area.get("areaName", [{}])[0].get("value", city)
    country = area.get("country", [{}])[0].get("value", "")

    temp_c = current.get("temp_C", "?")
    feels = current.get("FeelsLikeC", "?")
    humidity = current.get("humidity", "?")
    desc_list = current.get("weatherDesc", [{"value": "未知"}])
    desc = desc_list[0].get("value", "未知") if desc_list else "未知"
    wind_speed = current.get("windspeedKmph", "?")
    wind_dir = current.get("winddir16Point", "")
    visibility = current.get("visibility", "?")
    pressure = current.get("pressure", "?")

    return (
        f"📍 {area_name}, {country}\n"
        f"🌡 温度: {temp_c}°C(体感 {feels}°C)\n"
        f"🌤 天气: {desc}\n"
        f"💧 湿度: {humidity}%\n"
        f"💨 风: {wind_speed} km/h {wind_dir}\n"
        f"👁 能见度: {visibility} km\n"
        f"📊 气压: {pressure} hPa"
    )


# --- MCP JSON-RPC protocol ---

TOOLS = [
    {
        "name": "get_forecast",
        "description": (
            "查询某个城市的实时天气。输入城市名(中文或英文均可),"
            "返回温度、天气状况、湿度、风速等信息。"
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "城市名,如 '北京'、'Shanghai'、'Tokyo'",
                }
            },
            "required": ["city"],
        },
    }
]


def send(msg: dict) -> None:
    """Write one JSON-RPC message as a line to stdout."""
    sys.stdout.write(json.dumps(msg, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def handle(req: dict) -> dict | None:
    """Process one request; return a result dict, or None for notifications."""
    method = req.get("method", "")
    req_id = req.get("id")
    params = req.get("params", {})

    if method == "initialize":
        return {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "weather", "version": "0.1.0"},
        }

    if method == "notifications/initialized":
        return None  # notification — no response

    if method == "tools/list":
        return {"tools": TOOLS}

    if method == "tools/call":
        name = params.get("name", "")
        args = params.get("arguments", {})
        if name == "get_forecast":
            city = args.get("city", "")
            if not city:
                return {
                    "content": [{"type": "text", "text": "缺少参数: city"}],
                    "isError": True,
                }
            result = get_forecast(city)
            return {
                "content": [{"type": "text", "text": result}],
                "isError": False,
            }
        return {
            "content": [{"type": "text", "text": f"未知工具: {name}"}],
            "isError": True,
        }

    return {"error": {"code": -32601, "message": f"method not found: {method}"}}


def main() -> None:
    """Read JSON-RPC requests line-by-line from stdin, respond on stdout."""
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue
        result = handle(req)
        # Only respond to requests with an id (notifications have none).
        if result is not None and "id" in req:
            send({"jsonrpc": "2.0", "id": req["id"], "result": result})


if __name__ == "__main__":
    main()
