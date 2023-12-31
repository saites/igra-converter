<script setup lang="ts">
import { ref, onUnmounted } from 'vue'

export interface Props {
  open?: bool,
}

const props = withDefaults(defineProps<Props>(), {
  open: false,
})

const details = ref(null);
const summary = ref(null);
const content = ref(null);

let animation = null;
let isClosing = false;
let isExpanding = false;

onUnmounted(() => {
  if (animation) { animation.cancel() }
})

function click(e) {
  if (!details.value || !summary.value || !content.value) { return }
  details.value.style.overflow = 'hidden'
  if (isClosing || !details.value.open) {
    openDetails()
  } else if (isExpanding || details.value.open) {
    shrink()
  }
}

function shrink() {
  isClosing = true
  const startHeight = `${details.value.offsetHeight}px`
  const endHeight = `${summary.value.offsetHeight}px`
  if (animation) {
    animation.cancel()
  }

  animation = details.value.animate({
    height: [startHeight, endHeight],
  }, {
    duration: 400,
    easing: 'ease-out',
  })

  animation.onfinish = () => onAnimationFinish(false)
  animation.oncancel = () => isClosing = false
}

function openDetails() {
  details.value.style.height = `${details.value.offsetHeight}px`
  details.value.open = true
  requestAnimationFrame(() => expand())
}

function expand() {
  isExpanding = true
  const startHeight = `${details.value.offsetHeight}px`
  const endHeight = `${summary.value.offsetHeight + content.value.offsetHeight}px`

  if (animation) {
    animation.cancel()
  }

  animation = details.value.animate({
    height: [startHeight, endHeight]
  }, {
    duration: 400,
    easing: 'ease-out'
  })

  animation.onfinish = () => onAnimationFinish(true)
  animation.oncancel = () => isExpanding = false
}

function onAnimationFinish(open) {
  details.value.open = open
  animation = null
  isClosing = false
  isExpanding = false
  details.value.style.height = details.value.style.overflow = ''
}
</script>

<template>
  <details ref="details">
    <summary ref="summary" class="inline cursor-pointer" @click.prevent="click">
     <slot name="summary"></slot>
    </summary>
    <div ref="content"><slot name="content"></slot></div>
  </details>
</template>

