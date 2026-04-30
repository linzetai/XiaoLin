async function testChat() {
  const apiKey = process.env.DASHSCOPE_API_KEY;

  try {
    const res = await fetch('https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model: 'qwen-plus',
        messages: [
          { role: 'system', content: 'You are a helpful assistant.' },
          { role: 'user', content: '用一句话介绍你自己' },
        ],
      }),
    });

    if (!res.ok) {
      const err = await res.text();
      console.error(`❌ HTTP ${res.status}:`, err);
      return;
    }

    const data = await res.json();
    console.log('✅ 调用成功！');
    console.log('回复:', data.choices[0].message.content);
    console.log('Token 用量:', JSON.stringify(data.usage));
  } catch (err) {
    console.error('❌ 请求失败:', err.message);
  }
}

testChat();
