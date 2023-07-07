import MyGrid from './Grid.js'
import { createApp, ref, watchEffect, watch } from 'vue'

const url = "./validation_output.json"

export default {
  components: {
    MyGrid,
  },
  setup() {
    const registrationData = ref("")
    const validationResult = ref(null)
    const errMessage = ref(null)
    const validating = ref(false)
    const generating = ref(false)

    function validate() {
      validating.value = true;
    }

    function generate() {
      generating.value = true;
    }

    watch(validating,
        async () => {
          try {
            const response = await fetch('./validate', {
              method: "POST",
              cache: "no-cache", 
              headers: {
                "Content-Type": "application/json",
              },
              referrerPolicy: "no-referrer", 
              body: registrationData.value,
            });

            validationResult.value = await response.json()
          } catch(error) {
            errMessage.value = "Error: " + error;
            validationResult.value = null; 
          }
           
           validating.value = false;
        }, { immediate: false }
    )
    
    watch(generating,
        async () => {
          try {
            const response = await fetch('./generate', {
              method: "POST",
              cache: "no-cache", 
              headers: {
                "Content-Type": "application/json",
              },
              referrerPolicy: "no-referrer", 
              // body: JSON.stringify(generationOptions.value),
            });

            registrationData.value = JSON.stringify(await response.json())
            // validating.value = true;
          } catch(error) {
            errMessage.value = "Error: " + error;
            registrationData.value = ""; 
          }
           
          generating.value = false;
        }, { immediate: true }
    )

    return {
      registrationData,
      validationResult,
      errMessage,
      validating,
      generating,
      validate,
      generate,
    }
  },
  template: `
<div>
  <section>
      <textarea :disabled="generating || validating" v-model="registrationData"></textarea>
      <div>
        <button :disabled="generating || validating" @click="generate">Generate</button>
        <button :disabled="generating || validating" @click="validate">Validate</button>
      </div>
  </section>
  <div v-if="errMessage">
      {{errMessage}}
  </div>
  <my-grid v-else
      :results="validationResult?.results"
      :relevant="validationResult?.relevant"
  >
  </my-grid>
</div>
`
}

